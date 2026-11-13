#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod desktop;
use desktop::{DesktopHost, desktop_call, native_bootstrap, native_fetch, native_interrupt};
use desktop::{native_socket_close, native_socket_open, native_socket_send};

use cryzo_engine::{credentials::Credentials, service::Engine, tools::Operation};
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

type NativeResult = Result<Value, String>;

#[tauri::command]
fn bootstrap(engine: State<'_, Arc<Engine>>) -> NativeResult {
    Ok(
        json!({"sessions":engine.store.sessions().map_err(|error|error.to_string())?,"catalog":engine.catalog}),
    )
}

#[tauri::command]
fn list_sessions(engine: State<'_, Arc<Engine>>) -> NativeResult {
    engine
        .store
        .sessions()
        .and_then(|sessions| Ok(serde_json::to_value(sessions)?))
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn choose_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .map(|path| path.to_string())
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn create_session(
    engine: State<'_, Arc<Engine>>,
    directory: String,
    title: String,
) -> NativeResult {
    if title.len() > 500 {
        return Err("session title is too long".into());
    }
    engine
        .store
        .create_session(std::path::Path::new(&directory), &title)
        .and_then(|session| Ok(serde_json::to_value(session)?))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn events(engine: State<'_, Arc<Engine>>, session: String, after: i64) -> NativeResult {
    engine
        .store
        .events(&session, after)
        .and_then(|events| Ok(serde_json::to_value(events)?))
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn prompt(
    engine: State<'_, Arc<Engine>>,
    session: String,
    provider: String,
    model: String,
    id: String,
    text: String,
) -> NativeResult {
    engine
        .inner()
        .clone()
        .generate(session, provider, model, id, text)
        .await
        .map(|admitted| json!({"admitted":admitted}))
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn cancel(engine: State<'_, Arc<Engine>>, session: String) -> Result<(), String> {
    engine.cancel(&session).await;
    Ok(())
}

#[tauri::command]
async fn tool(
    engine: State<'_, Arc<Engine>>,
    session: String,
    operation: Operation,
    approval: Option<String>,
) -> NativeResult {
    engine
        .tool(&session, operation, approval.as_deref())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn decide(
    engine: State<'_, Arc<Engine>>,
    session: String,
    approval: String,
    allow: bool,
) -> NativeResult {
    engine
        .store
        .decide(&session, &approval, allow)
        .map(|()| json!({"recorded":true}))
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn save_credential(
    engine: State<'_, Arc<Engine>>,
    provider: String,
    secret: String,
) -> Result<(), String> {
    if !engine
        .catalog
        .providers
        .iter()
        .any(|entry| entry.id == provider)
    {
        return Err("unknown provider".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        Credentials::set(&provider, &secret).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Resized(_) = event {
                let _ = window.emit("cryzo:desktop", json!({"_tag":"WindowCaptionChanged","maximized":window.is_maximized().unwrap_or(false),"fullscreen":window.is_fullscreen().unwrap_or(false)}));
            }
            if let tauri::WindowEvent::Focused(focused) = event {
                let _ = window.emit(
                    "cryzo:desktop",
                    json!({"_tag":"WindowFocusChanged","focused":focused}),
                );
            }
        })
        .setup(|app| {
            #[cfg(target_os = "windows")]
            if let Some(window) = app.get_webview_window("main") {
                window.set_decorations(false)?;
            }
            let directory = std::env::var_os("CRYZO_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_data_dir()?);
            std::fs::create_dir_all(&directory)?;
            let engine = Arc::new(Engine::open(&directory.join("cryzo.sqlite"))?);
            app.manage(tauri::async_runtime::block_on(DesktopHost::start(
                &directory,
                engine.clone(),
            ))?);
            app.manage(engine);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            native_bootstrap,
            desktop_call,
            native_fetch,
            native_interrupt,
            native_socket_open,
            native_socket_send,
            native_socket_close,
            bootstrap,
            list_sessions,
            choose_directory,
            create_session,
            events,
            prompt,
            cancel,
            tool,
            decide,
            save_credential
        ])
        .run(tauri::generate_context!())
        .expect("Cryzo native host failed");
}
