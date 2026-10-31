//! Tauri host adapter for the unmodified upstream desktop renderer.
use cryzo_engine::{desktop_storage::DesktopStorage, service::Engine};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path, sync::Arc};
use tauri::{Emitter, Manager, State, ipc::Channel};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub struct DesktopHost {
    url: String,
    token: String,
    client: reqwest::Client,
    storage: DesktopStorage,
    requests: Mutex<HashMap<String, CancellationToken>>,
    zoom: Mutex<f64>,
    picked: Mutex<HashMap<String, Vec<std::path::PathBuf>>>,
    sockets: Mutex<HashMap<String, NativeSocket>>,
}

impl DesktopHost {
    pub async fn start(
        directory: &Path,
        engine: Arc<Engine>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let token = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let router = cryzo_engine::service::router(engine, token.clone());
        tauri::async_runtime::spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                eprintln!("native HTTP service stopped: {error}");
            }
        });
        Ok(Self {
            url,
            token,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()?,
            storage: DesktopStorage::open(&directory.join("desktop.sqlite"))?,
            requests: Mutex::new(HashMap::new()),
            zoom: Mutex::new(1.0),
            picked: Mutex::new(HashMap::new()),
            sockets: Mutex::new(HashMap::new()),
        })
    }

    fn first_launch(&self) -> Result<bool, String> {
        self.storage
            .call("StorageItems", &json!({"name":"native"}))
            .map(|value| value["items"]["onboarding"] != "complete")
            .map_err(|e| e.to_string())
    }

    fn setting(&self, key: &str) -> Result<Option<String>, String> {
        self.storage
            .call("StorageItems", &json!({"name":"native"}))
            .map(|value| value["items"][key].as_str().map(str::to_owned))
            .map_err(|e| e.to_string())
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.storage
            .call(
                "StorageUpdate",
                &json!({"name":"native","insert":{key:value},"remove":[]}),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn zoom_level(&self) -> Result<f64, String> {
        Ok(self
            .setting("zoom")?
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| [1.0, 1.1, 1.25, 1.5, 1.75, 2.0].contains(value))
            .unwrap_or(1.0))
    }
}

struct NativeSocket {
    pty: Arc<cryzo_engine::pty::Pty>,
    cancel: CancellationToken,
}
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SocketEvent {
    Open,
    Data { data: String },
    Cursor { cursor: u64 },
    Close { code: u16 },
}

#[tauri::command]
pub async fn native_socket_open(
    host: State<'_, DesktopHost>,
    engine: State<'_, Arc<Engine>>,
    id: String,
    url: String,
    channel: Channel<SocketEvent>,
) -> Result<(), String> {
    let url = reqwest::Url::parse(&url).map_err(|e| e.to_string())?;
    let mut expected = reqwest::Url::parse(&host.url).map_err(|e| e.to_string())?;
    expected
        .set_scheme("ws")
        .map_err(|_| "invalid local socket URL")?;
    if url.origin() != expected.origin() || !url.username().is_empty() || url.password().is_some() {
        return Err("socket is not the local engine".into());
    }
    let parts: Vec<_> = url.path().split('/').collect();
    if parts.len() != 5 || parts[1] != "api" || parts[2] != "pty" || parts[4] != "connect" {
        return Err("unsupported native socket endpoint".into());
    }
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let directory = std::path::PathBuf::from(
        query
            .get("location[directory]")
            .ok_or("missing terminal location")?,
    );
    let pty = engine
        .ptys
        .connect(
            parts[3],
            &directory,
            query.get("ticket").ok_or("missing terminal ticket")?,
        )
        .map_err(|e| e.to_string())?;
    let cursor = query
        .get("cursor")
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|e| e.to_string())?;
    let (chunks, watermark, mut output) = pty.attach(cursor).map_err(|e| e.to_string())?;
    let cancel = CancellationToken::new();
    {
        let mut sockets = host.sockets.lock().await;
        if sockets.contains_key(&id) {
            return Err("duplicate socket ID".into());
        }
        sockets.insert(
            id.clone(),
            NativeSocket {
                pty: pty.clone(),
                cancel: cancel.clone(),
            },
        );
    }
    let result:Result<(),String> = async {
        channel.send(SocketEvent::Open).map_err(|e|e.to_string())?;
        for chunk in chunks { channel.send(SocketEvent::Data {data:chunk.text}).map_err(|e|e.to_string())?; }
        channel.send(SocketEvent::Cursor {cursor:watermark}).map_err(|e|e.to_string())?;
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                next = output.recv() => {
                    match next {
                        Ok(chunk) if chunk.start >= watermark => { channel.send(SocketEvent::Data {data:chunk.text}).map_err(|e|e.to_string())?; }
                        Ok(_) => (),
                        Err(_) => break,
                    }
                }
                _ = tick.tick() => if pty.drained() && output.is_empty() { break; }
            }
        }
        Ok(())
    }.await;
    host.sockets.lock().await.remove(&id);
    let _ = channel.send(SocketEvent::Close {
        code: if result.is_ok() { 1000 } else { 1011 },
    });
    result
}
#[tauri::command]
pub async fn native_socket_send(
    host: State<'_, DesktopHost>,
    id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    let sockets = host.sockets.lock().await;
    let socket = sockets.get(&id).ok_or("terminal socket is closed")?;
    if socket.cancel.is_cancelled() {
        return Err("terminal socket is closing".into());
    }
    let pty = socket.pty.clone();
    drop(sockets);
    tauri::async_runtime::spawn_blocking(move || pty.write(&data))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn native_socket_close(host: State<'_, DesktopHost>, id: String) -> Result<(), String> {
    if let Some(socket) = host.sockets.lock().await.get(&id) {
        socket.cancel.cancel();
    }
    Ok(())
}

#[tauri::command]
pub async fn native_bootstrap(
    app: tauri::AppHandle,
    host: State<'_, DesktopHost>,
) -> Result<Value, String> {
    let factor = host.zoom_level()?;
    app.get_webview_window("main")
        .ok_or("main window is unavailable")?
        .set_zoom(factor)
        .map_err(|e| e.to_string())?;
    *host.zoom.lock().await = factor;
    Ok(json!({"url":host.url,"windowID":"main","firstLaunchPending":host.first_launch()?}))
}

#[tauri::command]
pub async fn desktop_call(
    app: tauri::AppHandle,
    host: State<'_, DesktopHost>,
    tag: String,
    payload: Value,
) -> Result<Value, String> {
    if tag.starts_with("Storage") || tag.starts_with("Drafts") {
        return host
            .storage
            .call(&tag, &payload)
            .map_err(|error| error.to_string());
    }
    let window = app
        .get_webview_window("main")
        .ok_or("main window is unavailable")?;
    let text = |key: &str| {
        payload[key]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("missing {key}"))
    };
    match tag.as_str() {
        "AppAwaitInitialization" | "AppReconnectService" => Ok(json!({"url":host.url})),
        "AppIsFirstLaunchOnboardingPending" => Ok(json!(host.first_launch()?)),
        "AppFinishFirstLaunchOnboarding" => {
            let directory = if payload["createDefaultProject"].as_bool() == Some(true) {
                let directory = app.path().document_dir().map_err(|e| e.to_string())?.join("Cryzo");
                std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
                Some(directory.to_string_lossy().into_owned())
            } else { None };
            host.storage.call("StorageUpdate", &json!({"name":"native","insert":{"onboarding":"complete"},"remove":[]})).map_err(|e| e.to_string())?;
            Ok(json!(directory))
        }
        "AppConsumeInitialDeepLinks" => Ok(json!([])),
        "WindowThemeReady" | "WindowShow" => { window.show().map_err(|e| e.to_string())?; Ok(Value::Null) }
        "WindowGetFocused" => window.is_focused().map(|v| json!(v)).map_err(|e| e.to_string()),
        "WindowGetFullscreen" => window.is_fullscreen().map(|v| json!(v)).map_err(|e| e.to_string()),
        "WindowSetFocus" => { window.set_focus().map_err(|e| e.to_string())?; Ok(Value::Null) }
        "MenuRunAction" => {
            match text("action")?.as_str() {
                "window.close" => window.close().map_err(|e| e.to_string())?,
                "window.minimize" => window.minimize().map_err(|e| e.to_string())?,
                "window.toggleMaximize" => {
                    if window.is_maximized().map_err(|e| e.to_string())? { window.unmaximize().map_err(|e| e.to_string())?; }
                    else { window.maximize().map_err(|e| e.to_string())?; }
                }
                "view.toggleFullscreen" => {
                    let fullscreen = !window.is_fullscreen().map_err(|e| e.to_string())?;
                    window.set_fullscreen(fullscreen).map_err(|e| e.to_string())?;
                    app.emit("cryzo:desktop", json!({"_tag":"WindowFullscreenChanged","fullscreen":fullscreen})).map_err(|e| e.to_string())?;
                }
                "view.reload" => window.eval("window.location.reload()").map_err(|e| e.to_string())?,
                "app.relaunch" => app.restart(),
                action => return Err(format!("Native menu action is not implemented: {action}")),
            }
            Ok(Value::Null)
        }
        "WindowGetZoomFactor" => Ok(json!(*host.zoom.lock().await)),
        "WindowGetZoomLevel" => Ok(json!(host.zoom_level()?)),
        "WindowGetPinchZoomEnabled" => Ok(json!(host.setting("pinch")?.as_deref() == Some("true"))),
        "WindowSetPinchZoomEnabled" => {
            let enabled = payload["enabled"].as_bool().ok_or("invalid pinch setting")?;
            host.set_setting("pinch", if enabled {"true"} else {"false"})?;
            app.emit("cryzo:desktop", json!({"_tag":"WindowPinchZoomChanged","enabled":enabled})).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "WindowSetZoomFactor" | "WindowSetZoomLevel" => {
            let factor = if tag == "WindowSetZoomLevel" { payload["level"].as_f64().ok_or("invalid zoom level")? }
                else { payload["factor"].as_f64().ok_or("invalid zoom factor")? };
            if !factor.is_finite() || !(0.2..=10.0).contains(&factor) { return Err("zoom is out of range".into()); }
            if tag == "WindowSetZoomLevel" && ![1.0,1.1,1.25,1.5,1.75,2.0].contains(&factor) { return Err("invalid desktop zoom level".into()); }
            window.set_zoom(factor).map_err(|e| e.to_string())?;
            *host.zoom.lock().await = factor;
            if tag == "WindowSetZoomLevel" { host.set_setting("zoom", &factor.to_string())?; }
            app.emit("cryzo:desktop", json!({"_tag":"WindowZoomChanged","factor":factor,"level":host.zoom_level()?})).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "WindowSetTitlebar" => {
            let theme = if payload["theme"]["mode"] == "dark" { tauri::Theme::Dark } else { tauri::Theme::Light };
            window.set_theme(Some(theme)).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "FilesOpenDirectoryPicker" => {
            let options = payload["options"].clone();
            tauri::async_runtime::spawn_blocking(move || {
                let mut picker = app.dialog().file();
                if let Some(title) = options["title"].as_str() { picker = picker.set_title(title); }
                if let Some(path) = options["defaultPath"].as_str() { picker = picker.set_directory(path); }
                if options["multiple"].as_bool() == Some(true) {
                    json!(picker.blocking_pick_folders().map(|paths| paths.into_iter().map(|p| p.to_string()).collect::<Vec<_>>()))
                } else { json!(picker.blocking_pick_folder().map(|p| p.to_string())) }
            }).await.map_err(|e| e.to_string())
        }
        "FilesOpenFilePicker" => {
            let options = payload["options"].clone();
            let paths = tauri::async_runtime::spawn_blocking(move || {
                let mut picker = app.dialog().file();
                if let Some(title) = options["title"].as_str() { picker = picker.set_title(title); }
                if let Some(path) = options["defaultPath"].as_str() { picker = picker.set_directory(path); }
                if options["multiple"].as_bool() == Some(true) { picker.blocking_pick_files() }
                else { picker.blocking_pick_file().map(|path| vec![path]) }
            }).await.map_err(|e| e.to_string())?;
            let Some(paths) = paths else { return Ok(Value::Null); };
            let paths = paths.into_iter().map(|path| path.into_path().map_err(|e| e.to_string()).and_then(|path| path.canonicalize().map_err(|e| e.to_string())))
                .collect::<Result<Vec<_>,_>>()?;
            let mut files = Vec::new();
            for path in &paths {
                let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
                // Attachment reads enforce their own 20 MB limit in FilesReadPickedFile; path-only picks
                // (Marketing media uploads, streamed by the engine) may be larger.
                if !metadata.is_file() || metadata.len() > 1_073_741_824 { return Err("selected file exceeds the 1 GB limit".into()); }
                files.push(json!({"path":path.to_string_lossy(),"name":path.file_name().unwrap_or_default().to_string_lossy(),"size":metadata.len()}));
            }
            let token = uuid::Uuid::new_v4().to_string();
            host.picked.lock().await.insert(token.clone(), paths);
            Ok(json!({"token":token,"files":files}))
        }
        "FilesReadPickedFile" => {
            use tokio::io::AsyncReadExt;
            let token = text("token")?;
            let path = std::path::PathBuf::from(text("path")?).canonicalize().map_err(|e| e.to_string())?;
            if !host.picked.lock().await.get(&token).is_some_and(|paths| paths.contains(&path)) {
                return Err("file was not selected by this window".into());
            }
            let file = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            file.take(20_000_001).read_to_end(&mut bytes).await.map_err(|e| e.to_string())?;
            if bytes.len() > 20_000_000 { return Err("selected file exceeds the attachment limit".into()); }
            Ok(json!(bytes))
        }
        "FilesReleasePickedFiles" => {
            host.picked.lock().await.remove(&text("token")?);
            Ok(Value::Null)
        }
        "FilesSaveFile" => {
            let content = text("content")?;
            let options = payload["options"].clone();
            let path = tauri::async_runtime::spawn_blocking(move || {
                let mut picker = app.dialog().file();
                if let Some(title) = options["title"].as_str() { picker = picker.set_title(title); }
                if let Some(path) = options["defaultPath"].as_str() {
                    let path = std::path::Path::new(path);
                    if let Some(parent) = path.parent() { picker = picker.set_directory(parent); }
                    if let Some(name) = path.file_name() { picker = picker.set_file_name(name.to_string_lossy()); }
                }
                picker.blocking_save_file()
            }).await.map_err(|e| e.to_string())?;
            let Some(path) = path else { return Ok(json!(false)); };
            tokio::fs::write(path.into_path().map_err(|e| e.to_string())?, content).await.map_err(|e| e.to_string())?;
            Ok(json!(true))
        }
        // No installed external plugins have been imported into this native profile yet.
        "PluginList" => Ok(json!([
            {"id":"ssh","builtin":true,"enabled":true,"server":true,"error":"Native SSH adapter is not available in this build."},
            {"id":"wsl","builtin":true,"enabled":true,"server":true,"error":"Native WSL adapter is not available in this build."}
        ])),
        "PluginMenubarItems" => Ok(json!([])),
        "PluginGuis" => Ok(json!({"plugins":[],"reserved":[]})),
        "PluginSubscribe" => Ok(json!({"available":false})),
        "PluginConfigure" => Ok(Value::Null),
        "PluginCall" => Err(json!({"code":"unavailable","message":"This native plugin adapter is not available yet."}).to_string()),
        "AppRecordFatalRendererError" => {
            // Log the error message, never serialized renderer state, prompts, or credentials.
            eprintln!("renderer failure: {}", payload["error"]["error"].as_str().unwrap_or("unknown"));
            Ok(Value::Null)
        }
        "FilesOpenExternal" | "FilesOpenBrowser" => {
            // Same allowlist as the Electron host: only web links, and mail links for external opens.
            let url = tauri::Url::parse(&text("url")?).map_err(|e| e.to_string())?;
            let allowed = match url.scheme() {
                "http" | "https" => true,
                "mailto" => tag == "FilesOpenExternal",
                _ => false,
            };
            if !allowed {
                return Ok(json!(false));
            }
            // The parsed URL goes to the OS handler as one argument, never through a shell.
            #[cfg(windows)]
            let mut command = {
                let mut command = std::process::Command::new("rundll32");
                command.arg("url.dll,FileProtocolHandler");
                command
            };
            #[cfg(target_os = "macos")]
            let mut command = std::process::Command::new("open");
            #[cfg(all(unix, not(target_os = "macos")))]
            let mut command = std::process::Command::new("xdg-open");
            Ok(json!(command.arg(url.as_str()).spawn().is_ok()))
        }
        "AppRelaunch" => { app.restart() }
        "AppCheckAppExists" => Ok(json!(resolve_application(&text("appName")?).is_some())),
        "AppResolveAppPath" => Ok(json!(resolve_application(&text("appName")?))),
        // These are Electron-specific painting hints; WebView2 owns these settings.
        "AppSetNativeTranslations" | "WindowSetTrafficLightsPosition" | "AppSetBackgroundColor" => Ok(Value::Null),
        _ => Err(format!("Native desktop operation is not implemented: {tag}")),
    }
}

fn resolve_application(name: &str) -> Option<String> {
    if name.is_empty() || name.contains(['/', '\\', ':']) {
        return None;
    }
    let paths = std::env::var_os("PATH")?;
    let extensions: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::split_paths(&paths)
        .flat_map(|path| {
            extensions
                .iter()
                .map(move |extension| path.join(format!("{name}{extension}")))
        })
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum FetchEvent {
    Head {
        status: u16,
        headers: Vec<(String, String)>,
    },
    Data {
        bytes: Vec<u8>,
    },
    End,
    Error {
        message: String,
    },
}

#[derive(Deserialize)]
pub struct NativeRequest {
    id: String,
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[tauri::command]
pub async fn native_fetch(
    host: State<'_, DesktopHost>,
    request: NativeRequest,
    channel: Channel<FetchEvent>,
) -> Result<(), String> {
    let url = reqwest::Url::parse(&request.url).map_err(|e| e.to_string())?;
    let base = reqwest::Url::parse(&host.url).map_err(|e| e.to_string())?;
    if url.origin() != base.origin() || !url.username().is_empty() || url.password().is_some() {
        return Err("native fetch is restricted to the local engine".into());
    }
    if request.body.len() > 20_000_000 {
        return Err("request body is too large".into());
    }
    let method =
        reqwest::Method::from_bytes(request.method.as_bytes()).map_err(|e| e.to_string())?;
    let mut builder = host
        .client
        .request(method, url)
        .bearer_auth(&host.token)
        .body(request.body);
    for (name, value) in request.headers {
        // Never forward renderer credentials, origin, host, cookies, or hop-by-hop headers.
        if [
            "content-type",
            "accept",
            "last-event-id",
            "x-opencode-ticket",
        ]
        .contains(&name.to_ascii_lowercase().as_str())
        {
            builder = builder.header(name, value);
        }
    }
    let cancel = CancellationToken::new();
    {
        let mut requests = host.requests.lock().await;
        if requests.contains_key(&request.id) {
            return Err("duplicate native request ID".into());
        }
        requests.insert(request.id.clone(), cancel.clone());
    }
    let result: Result<(), String> = async {
        let response = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            result = builder.send() => result.map_err(|e| e.to_string())?,
        };
        let headers = response.headers().iter().filter_map(|(name, value)| value.to_str().ok().map(|value| (name.to_string(),value.to_owned()))).collect();
        channel.send(FetchEvent::Head { status: response.status().as_u16(), headers }).map_err(|e| e.to_string())?;
        let mut stream = response.bytes_stream();
        loop {
            let next = tokio::select! { _ = cancel.cancelled() => return Ok(()), next = stream.next() => next };
            let Some(bytes) = next else { break; };
            channel.send(FetchEvent::Data { bytes: bytes.map_err(|e| e.to_string())?.to_vec() }).map_err(|e| e.to_string())?;
        }
        channel.send(FetchEvent::End).map_err(|e| e.to_string())
    }.await;
    host.requests.lock().await.remove(&request.id);
    if let Err(message) = &result {
        let _ = channel.send(FetchEvent::Error {
            message: message.clone(),
        });
    }
    result
}

#[tauri::command]
pub async fn native_interrupt(host: State<'_, DesktopHost>, id: String) -> Result<(), String> {
    if let Some(cancel) = host.requests.lock().await.get(&id) {
        cancel.cancel();
    }
    Ok(())
}
