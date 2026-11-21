#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod browser;
mod desktop;
#[cfg(target_os = "linux")]
mod webkit_linux;
#[cfg(target_os = "macos")]
mod webkit_mac;
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

/// Brings the main window back from the tray, the taskbar or a second launch.
fn reveal(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The tray icon that keeps Cryzo reachable after its window closes; Quit really quits.
fn tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    let open = MenuItem::with_id(app, "open", "Open Cryzo", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Cryzo", true, None::<&str>)?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Cryzo")
        .menu(&Menu::with_items(app, &[&open, &quit])?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => reveal(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn main() {
    // Before anything else starts: apps opened from the Dock or an app menu need the login shell's PATH.
    cryzo_engine::shell_env::adopt();
    // Started at sign-in: open into the tray rather than in front of the user.
    let hidden = std::env::args().any(|argument| argument == "--background");
    tauri::Builder::default()
        // A second launch (from the Start menu, say) shows the running Cryzo instead of starting another.
        .plugin(tauri_plugin_single_instance::init(|app, _arguments, _directory| reveal(app)))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            // Closing the window keeps Cryzo running in the tray, so remote control stays connected.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
                && window
                    .app_handle()
                    .try_state::<DesktopHost>()
                    .is_some_and(|host| host.keeps_running())
            {
                api.prevent_close();
                let _ = window.hide();
            }
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
        // The composer's mic needs the microphone. Only Cryzo's own interface gets it; pages in
        // the browser pane keep WebView2's default and Windows' own microphone setting applies.
        .on_permission_request(|webview, kind| {
            use tauri::webview::{PermissionKind, PermissionResponse};
            if webview.label() == "main" && matches!(kind, PermissionKind::Microphone) {
                PermissionResponse::Allow
            } else {
                PermissionResponse::Default
            }
        })
        .setup(move |app| {
            #[cfg(target_os = "windows")]
            if let Some(window) = app.get_webview_window("main") {
                window.set_decorations(false)?;
            }
            if hidden && let Some(window) = app.get_webview_window("main") {
                window.hide()?;
            }
            tray(app)?;
            let directory = std::env::var_os("CRYZO_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_data_dir()?);
            std::fs::create_dir_all(&directory)?;
            let engine = Arc::new(Engine::open(&directory.join("cryzo.sqlite"))?);
            engine.install_pane(Arc::new(browser::PreviewPane(app.handle().clone())));
            let host = tauri::async_runtime::block_on(DesktopHost::start(
                &directory,
                engine.clone(),
                hidden,
            ))?;
            tauri::async_runtime::block_on(host.start_at_sign_in(app.handle()));
            app.manage(host);
            app.manage(engine);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            browser::browser_pane,
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
        .build(tauri::generate_context!())
        .expect("Cryzo native host failed")
        .run(|app, event| match event {
            // Dev servers end with the app; their job objects also end them after a crash.
            tauri::RunEvent::Exit => {
                if let Some(engine) = app.try_state::<Arc<Engine>>() {
                    engine.previews.stop_all();
                }
            }
            // macOS: clicking the Dock icon brings the hidden window back.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => reveal(app),
            _ => {}
        });
}
