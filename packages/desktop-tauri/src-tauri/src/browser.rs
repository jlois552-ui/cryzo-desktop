//! The browser pane: child webviews (one per tab) inside the main window that show dev servers and
//! other pages, with annotation and an element editor. The page gets no IPC access. The picker
//! reports a pick by starting a navigation to a sentinel URL, which `on_navigation` reads and
//! cancels, then forwards to the renderer as a desktop event.
use serde_json::{Value, json};
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, webview::WebviewBuilder};

const LABEL: &str = "browser";
const SENTINEL: &str = "cryzo-note.invalid";
const PICKER: &str = include_str!("browser-picker.js");

/// Lets the engine show a dev server it started: the renderer opens the pane at the URL.
pub struct PreviewPane(pub tauri::AppHandle);

impl cryzo_engine::preview::PaneHost for PreviewPane {
    fn open(&self, directory: &std::path::Path, url: &str) {
        let _ = self.0.emit(
            "cryzo:desktop",
            json!({"_tag":"BrowserOpen","directory":directory,"url":url}),
        );
    }

    /// Runs on the pane's own WebView2 through `CallDevToolsProtocolMethod`, so it reaches only
    /// that page: no debugging port is opened, and Cryzo's own interface stays out of reach.
    fn cdp(&self, method: &str, params: Value) -> cryzo_engine::preview::Reply {
        let (sender, answer) = tokio::sync::oneshot::channel::<Result<Value, String>>();
        let started = match pane() {
            Some(webview) => devtools(&webview, method.to_owned(), params.to_string(), sender),
            None => Err("the browser pane isn't open".into()),
        };
        Box::pin(async move {
            started.map_err(anyhow::Error::msg)?;
            let reply = tokio::time::timeout(std::time::Duration::from_secs(20), answer)
                .await
                .map_err(|_| anyhow::anyhow!("the page didn't answer within 20 seconds"))?
                .map_err(|_| anyhow::anyhow!("the browser pane closed"))?;
            reply.map_err(anyhow::Error::msg)
        })
    }
}

#[cfg(windows)]
fn devtools(
    webview: &tauri::Webview,
    method: String,
    params: String,
    sender: tokio::sync::oneshot::Sender<Result<Value, String>>,
) -> Result<(), String> {
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::HSTRING;
    // The completion handler takes the sender; if the call never starts, it is dropped and the
    // caller hears that the pane closed.
    webview
        .with_webview(move |platform| {
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                move |error, reply: String| {
                    let result = error.map_err(|error| error.message()).and_then(|()| {
                        serde_json::from_str(&reply).map_err(|error| error.to_string())
                    });
                    let _ = sender.send(result);
                    Ok(())
                },
            ));
            // SAFETY: COM calls on the pane's controller, on the thread Tauri runs this closure on.
            unsafe {
                if let Ok(core) = platform.controller().CoreWebView2() {
                    let _ = core.CallDevToolsProtocolMethod(
                        &HSTRING::from(method),
                        &HSTRING::from(params),
                        &handler,
                    );
                }
            }
        })
        .map_err(|error| error.to_string())
}

#[cfg(not(windows))]
fn devtools(
    _webview: &tauri::Webview,
    _method: String,
    _params: String,
    _sender: tokio::sync::oneshot::Sender<Result<Value, String>>,
) -> Result<(), String> {
    Err("controlling the browser pane is only available on Windows for now".into())
}

/// The visible page as base64 PNG, taken while the picker still draws the marks a note points at.
/// Notes still go out without it when the pane can't take one.
async fn screenshot() -> Option<String> {
    let webview = pane()?;
    let (sender, answer) = tokio::sync::oneshot::channel();
    devtools(
        &webview,
        "Page.captureScreenshot".into(),
        json!({"format":"png"}).to_string(),
        sender,
    )
    .ok()?;
    let reply = tokio::time::timeout(std::time::Duration::from_secs(5), answer)
        .await
        .ok()?
        .ok()?
        .ok()?;
    reply["data"].as_str().map(str::to_owned)
}

/// The pane's tabs, each its own child webview, kept from creation: looking a child webview up by
/// label does not find it. Only the active tab is shown; the agent's page tools act on it.
static PANE: std::sync::Mutex<Tabs> = std::sync::Mutex::new(Tabs {
    list: Vec::new(),
    active: None,
});

struct Tabs {
    list: Vec<(String, tauri::Webview)>,
    active: Option<String>,
}

impl Tabs {
    /// Makes `id` the active tab, adding it when new, and returns the tabs to hide.
    fn select(&mut self, id: &str, webview: &tauri::Webview) -> Vec<tauri::Webview> {
        if !self.list.iter().any(|(tab, _)| tab == id) {
            self.list.push((id.to_owned(), webview.clone()));
        }
        self.active = Some(id.to_owned());
        self.list
            .iter()
            .filter(|(tab, _)| tab != id)
            .map(|(_, webview)| webview.clone())
            .collect()
    }

    /// Forgets tab `id`; another tab becomes active only when the renderer shows it.
    fn remove(&mut self, id: &str) -> Option<tauri::Webview> {
        let index = self.list.iter().position(|(tab, _)| tab == id)?;
        if self.active.as_deref() == Some(id) {
            self.active = None;
        }
        Some(self.list.remove(index).1)
    }

    fn get(&self, id: &str) -> Option<tauri::Webview> {
        self.list
            .iter()
            .find(|(tab, _)| tab == id)
            .map(|(_, webview)| webview.clone())
    }
}

fn pane() -> Option<tauri::Webview> {
    let tabs = PANE.lock().ok()?;
    tabs.get(tabs.active.as_deref()?)
}

/// The renderer's pane calls, straight from the Tauri bootstrap (see `src/native.ts`).
#[tauri::command]
pub async fn browser_pane(
    app: tauri::AppHandle,
    tag: String,
    payload: Value,
) -> Result<Value, String> {
    let result = match tag.as_str() {
        "BrowserShow" => show(&app, &payload),
        // A still of the page, drawn by the renderer while a menu overlaps the native page.
        "BrowserStill" => Ok(json!({ "screenshot": screenshot().await })),
        _ => call(&tag, &payload),
    };
    result.inspect_err(|error| eprintln!("browser pane: {tag} failed: {error}"))
}

/// Shows tab `tab` at `rect` (logical pixels in the main window), creating it on first use, and
/// hides the other tabs.
fn show(app: &tauri::AppHandle, payload: &Value) -> Result<Value, String> {
    let (position, size) = rect(payload)?;
    let id = tab_id(payload)?;
    let url = payload["url"].as_str().map(page).transpose()?;
    let existing = PANE.lock().map_err(|e| e.to_string())?.get(&id);
    let webview = match existing {
        Some(webview) => {
            webview.set_position(position).map_err(|e| e.to_string())?;
            webview.set_size(size).map_err(|e| e.to_string())?;
            if let Some(url) = url {
                webview.navigate(url).map_err(|e| e.to_string())?;
            }
            webview
        }
        None => create(
            app,
            &id,
            url.unwrap_or(page("about:blank")?),
            position,
            size,
        )?,
    };
    let others = PANE
        .lock()
        .map_err(|e| e.to_string())?
        .select(&id, &webview);
    for other in others {
        let _ = other.hide();
    }
    webview.show().map_err(|e| e.to_string())?;
    Ok(Value::Null)
}

fn create(
    app: &tauri::AppHandle,
    id: &str,
    url: tauri::Url,
    position: LogicalPosition<f64>,
    size: LogicalSize<f64>,
) -> Result<tauri::Webview, String> {
    let window = app.get_window("main").ok_or("main window is unavailable")?;
    let notes = app.clone();
    let loads = app.clone();
    let titles = app.clone();
    let popups = app.clone();
    let builder = WebviewBuilder::new(format!("{LABEL}-{id}"), WebviewUrl::External(url))
        .initialization_script(PICKER)
        .on_navigation(move |url| {
            if url.host_str() != Some(SENTINEL) {
                return matches!(url.scheme(), "http" | "https" | "about");
            }
            match message(url) {
                Some(Message::Notes(sent)) => {
                    let emitter = notes.clone();
                    tauri::async_runtime::spawn(async move {
                        let screenshot = screenshot().await;
                        if let Some(webview) = pane() {
                            let _ = webview
                                .eval("window.__cryzoPicker&&window.__cryzoPicker.captured()");
                        }
                        let _ = emitter.emit(
                            "cryzo:desktop",
                            json!({"_tag":"BrowserNotes","notes":sent,"screenshot":screenshot}),
                        );
                    });
                }
                Some(Message::Selection(selection)) => {
                    let _ = notes.emit(
                        "cryzo:desktop",
                        json!({"_tag":"BrowserSelection","selection":selection}),
                    );
                }
                None => {}
            }
            false
        })
        .on_page_load(move |webview, load| {
            // WebView2 also reports the cancelled sentinel navigation; the page stays.
            if load.url().host_str() == Some(SENTINEL) {
                return;
            }
            let tag = match load.event() {
                tauri::webview::PageLoadEvent::Started => "BrowserLoading",
                tauri::webview::PageLoadEvent::Finished => "BrowserLoaded",
            };
            let _ = loads.emit(
                "cryzo:desktop",
                json!({"_tag":tag,"tab":tab_of(&webview),"url":load.url().as_str()}),
            );
        })
        .on_document_title_changed(move |webview, title| {
            let _ = titles.emit(
                "cryzo:desktop",
                json!({"_tag":"BrowserTitle","tab":tab_of(&webview),"title":title}),
            );
        })
        // Links that open a new window open as a new tab of the pane instead.
        .on_new_window(move |url, _features| {
            if matches!(url.scheme(), "http" | "https") {
                let _ = popups.emit(
                    "cryzo:desktop",
                    json!({"_tag":"BrowserNewTab","url":url.as_str()}),
                );
            }
            tauri::webview::NewWindowResponse::Deny
        });
    window
        .add_child(builder, position, size)
        .map_err(|e| e.to_string())
}

fn tab_of(webview: &tauri::Webview) -> String {
    webview
        .label()
        .strip_prefix(LABEL)
        .and_then(|rest| rest.strip_prefix('-'))
        .unwrap_or_default()
        .to_owned()
}

/// Tab ids come from the renderer and become part of a webview label.
fn tab_id(payload: &Value) -> Result<String, String> {
    payload["tab"]
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 64
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        .map(str::to_owned)
        .ok_or_else(|| "invalid tab".into())
}

fn call(tag: &str, payload: &Value) -> Result<Value, String> {
    if tag == "BrowserTabClose" {
        let closed = PANE
            .lock()
            .map_err(|e| e.to_string())?
            .remove(&tab_id(payload)?);
        if let Some(webview) = closed {
            webview.close().map_err(|e| e.to_string())?;
        }
        return Ok(Value::Null);
    }
    let Some(webview) = pane() else {
        return Ok(Value::Null);
    };
    let picker = |script: String| {
        webview
            .eval(format!(
                "window.__cryzoPicker&&window.__cryzoPicker.{script}"
            ))
            .map_err(|e| e.to_string())
    };
    match tag {
        "BrowserHide" => webview.hide().map_err(|e| e.to_string())?,
        "BrowserBounds" => {
            let (position, size) = rect(payload)?;
            webview.set_position(position).map_err(|e| e.to_string())?;
            webview.set_size(size).map_err(|e| e.to_string())?;
        }
        "BrowserNavigate" => webview
            .navigate(page(payload["url"].as_str().ok_or("missing url")?)?)
            .map_err(|e| e.to_string())?,
        "BrowserBack" => webview.eval("history.back()").map_err(|e| e.to_string())?,
        "BrowserForward" => webview
            .eval("history.forward()")
            .map_err(|e| e.to_string())?,
        "BrowserReload" => webview
            .eval("location.reload()")
            .map_err(|e| e.to_string())?,
        // The picker runs in the page; these only switch and steer it. Every value reaches the page
        // as JSON, and the picker checks style properties against its own list.
        "BrowserAnnotate" => {
            let on = payload["enabled"].as_bool().unwrap_or(false);
            let batch = payload["batch"].as_bool().unwrap_or(false);
            let labels = payload["labels"].clone();
            picker(format!("set({on},{batch},{labels})"))?;
        }
        "BrowserSendBatch" => picker("flush()".into())?,
        "BrowserStyle" => {
            let property = Value::from(payload["property"].as_str().ok_or("missing property")?);
            let value = Value::from(payload["value"].as_str().ok_or("missing value")?);
            picker(format!("style({property},{value})"))?;
        }
        "BrowserEditReset" => picker("reset()".into())?,
        "BrowserEditCancel" => picker("cancelEdit()".into())?,
        "BrowserEditSend" => {
            let comment = Value::from(payload["comment"].as_str().unwrap_or_default());
            picker(format!("sendEdit({comment})"))?;
        }
        _ => return Err(format!("unknown browser call {tag}")),
    }
    Ok(Value::Null)
}

/// Only web pages: typed addresses without a scheme are treated as `http://` for local dev servers.
fn page(url: &str) -> Result<tauri::Url, String> {
    let url = url.trim();
    let candidate = if url.contains("://") || url == "about:blank" {
        url.to_owned()
    } else {
        format!("http://{url}")
    };
    let parsed = tauri::Url::parse(&candidate).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https" | "about") {
        return Err("Only http and https pages can open in the browser pane.".into());
    }
    Ok(parsed)
}

fn rect(payload: &Value) -> Result<(LogicalPosition<f64>, LogicalSize<f64>), String> {
    let number = |key: &str| {
        payload[key]
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .ok_or_else(|| format!("invalid {key}"))
    };
    Ok((
        LogicalPosition::new(number("x")?, number("y")?),
        LogicalSize::new(number("width")?.max(1.0), number("height")?.max(1.0)),
    ))
}

/// What the picker sent, from the sentinel URL's fragment: notes as a JSON array, or the element
/// the inspector edits as `{"selection": ...}`. Capped in size and count.
enum Message {
    Notes(Value),
    Selection(Value),
}

fn message(url: &tauri::Url) -> Option<Message> {
    let fragment = url.fragment()?;
    if fragment.len() > 256 * 1024 {
        return None;
    }
    let value: Value = serde_json::from_str(&percent_decode(fragment)?).ok()?;
    if let Some(selection) = value.get("selection").filter(|value| value.is_object()) {
        return Some(Message::Selection(selection.clone()));
    }
    let notes = value.as_array()?;
    (!notes.is_empty() && notes.len() <= 50 && notes.iter().all(Value::is_object))
        .then(|| Message::Notes(value.clone()))
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_pages_open() {
        assert_eq!(
            page("localhost:5173").unwrap().as_str(),
            "http://localhost:5173/"
        );
        assert!(page("https://example.com").is_ok());
        assert!(page("file:///C:/Windows/win.ini").is_err());
        assert!(page("javascript:alert(1)").is_err());
    }

    #[test]
    fn messages_come_only_from_a_well_formed_sentinel_fragment() {
        let url =
            |fragment: &str| tauri::Url::parse(&format!("https://{SENTINEL}/#{fragment}")).unwrap();
        let notes = |fragment: &str| match message(&url(fragment)) {
            Some(Message::Notes(notes)) => Some(notes),
            _ => None,
        };
        assert_eq!(
            notes("%5B%7B%22kind%22%3A%22element%22%7D%5D"),
            Some(json!([{"kind":"element"}]))
        );
        assert_eq!(notes("not-json"), None);
        assert_eq!(notes("%7B%7D"), None);
        assert_eq!(notes("%5B1%5D"), None);
        assert_eq!(notes("%5B%5D"), None);
        let selection = url("%7B%22selection%22%3A%7B%22element%22%3A%7B%7D%7D%7D");
        assert!(matches!(
            message(&selection),
            Some(Message::Selection(value)) if value == json!({"element":{}})
        ));
        assert!(message(&url("%7B%22selection%22%3A1%7D")).is_none());
    }
}
