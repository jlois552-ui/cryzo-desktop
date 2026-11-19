//! The browser pane: a child webview inside the main window that shows dev servers and other pages,
//! with Cursor-style annotation. The page gets no IPC access. The picker reports a pick by starting
//! a navigation to a sentinel URL, which `on_navigation` reads and cancels, then forwards to the
//! renderer as a desktop event.
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

/// The pane, kept from creation: looking a child webview up by label does not find it.
static PANE: std::sync::Mutex<Option<tauri::Webview>> = std::sync::Mutex::new(None);

fn pane() -> Option<tauri::Webview> {
    PANE.lock().ok()?.clone()
}

/// The renderer's pane calls, straight from the Tauri bootstrap (see `src/native.ts`).
#[tauri::command]
pub async fn browser_pane(
    app: tauri::AppHandle,
    tag: String,
    payload: Value,
) -> Result<Value, String> {
    let result = if tag == "BrowserShow" {
        show(&app, &payload).await
    } else {
        call(&tag, &payload)
    };
    result.inspect_err(|error| eprintln!("browser pane: {tag} failed: {error}"))
}

/// Shows the pane at `rect` (logical pixels in the main window), creating it on first use.
async fn show(app: &tauri::AppHandle, payload: &Value) -> Result<Value, String> {
    let (position, size) = rect(payload)?;
    if let Some(webview) = pane() {
        webview.set_position(position).map_err(|e| e.to_string())?;
        webview.set_size(size).map_err(|e| e.to_string())?;
        webview.show().map_err(|e| e.to_string())?;
        if let Some(url) = payload["url"].as_str() {
            webview.navigate(page(url)?).map_err(|e| e.to_string())?;
        }
        return Ok(Value::Null);
    }
    let url = page(payload["url"].as_str().unwrap_or("about:blank"))?;
    let window = app.get_window("main").ok_or("main window is unavailable")?;
    let emitter = app.clone();
    let loads = app.clone();
    let builder = WebviewBuilder::new(LABEL, WebviewUrl::External(url))
        .initialization_script(PICKER)
        .on_navigation(move |url| {
            if url.host_str() != Some(SENTINEL) {
                return matches!(url.scheme(), "http" | "https" | "about");
            }
            if let Some(notes) = notes(url) {
                let emitter = emitter.clone();
                tauri::async_runtime::spawn(async move {
                    let screenshot = screenshot().await;
                    if let Some(webview) = pane() {
                        let _ =
                            webview.eval("window.__cryzoPicker&&window.__cryzoPicker.captured()");
                    }
                    let _ = emitter.emit(
                        "cryzo:desktop",
                        json!({"_tag":"BrowserNotes","notes":notes,"screenshot":screenshot}),
                    );
                });
            }
            false
        })
        .on_page_load(move |webview, load| {
            if matches!(load.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = loads.emit(
                    "cryzo:desktop",
                    json!({"_tag":"BrowserLoaded","url":load.url().as_str()}),
                );
                let _ = webview;
            }
        });
    let webview = window
        .add_child(builder, position, size)
        .map_err(|e| e.to_string())?;
    if let Ok(mut pane) = PANE.lock() {
        *pane = Some(webview);
    }
    Ok(Value::Null)
}

fn call(tag: &str, payload: &Value) -> Result<Value, String> {
    let Some(webview) = pane() else {
        return Ok(Value::Null);
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
        "BrowserReload" => webview
            .eval("location.reload()")
            .map_err(|e| e.to_string())?,
        // The picker runs in the page; these only switch it, so the page learns nothing it could misuse.
        "BrowserAnnotate" => {
            let on = payload["enabled"].as_bool().unwrap_or(false);
            let batch = payload["batch"].as_bool().unwrap_or(false);
            let draw = payload["draw"].as_bool().unwrap_or(false);
            webview
                .eval(format!(
                    "window.__cryzoPicker&&window.__cryzoPicker.set({on},{batch},{draw})"
                ))
                .map_err(|e| e.to_string())?;
        }
        "BrowserSendBatch" => webview
            .eval("window.__cryzoPicker&&window.__cryzoPicker.flush()")
            .map_err(|e| e.to_string())?,
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

/// The picker's notes from the sentinel URL's fragment: a JSON array, capped in size and count.
fn notes(url: &tauri::Url) -> Option<Value> {
    let fragment = url.fragment()?;
    if fragment.len() > 256 * 1024 {
        return None;
    }
    let decoded = percent_decode(fragment)?;
    let value: Value = serde_json::from_str(&decoded).ok()?;
    let notes = value.as_array()?;
    (notes.len() <= 50 && notes.iter().all(Value::is_object)).then(|| value.clone())
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
    fn notes_come_only_from_a_well_formed_sentinel_fragment() {
        let url =
            |fragment: &str| tauri::Url::parse(&format!("https://{SENTINEL}/#{fragment}")).unwrap();
        let good = url("%5B%7B%22kind%22%3A%22element%22%7D%5D");
        assert_eq!(notes(&good), Some(json!([{"kind":"element"}])));
        assert_eq!(notes(&url("not-json")), None);
        assert_eq!(notes(&url("%7B%7D")), None);
        assert_eq!(notes(&url("%5B1%5D")), None);
    }
}
