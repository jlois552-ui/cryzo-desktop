//! The preview of the app being built, as in ChatGPT Sites and Cursor: the agent starts the
//! project's dev server (for example `npm run dev`), Cryzo waits until it answers and shows it in
//! the built-in browser pane. One preview runs per project; starting it again replaces it.
//!
//! A dev server never exits, so it can't run through the `shell` tool. Its whole process tree
//! (npm, node, workerd) is tied to a Windows job object that ends with it, and with Cryzo, even
//! after a crash. Nothing is saved, so a preview is never restarted on its own after a restart.
use crate::service::Engine;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

const KEPT_LINES: usize = 400;
const READY_WITHIN: Duration = Duration::from_secs(90);

/// The running previews, by project folder.
#[derive(Default)]
pub struct Previews(Mutex<HashMap<PathBuf, Running>>);

struct Running {
    child: tokio::process::Child,
    log: Arc<Mutex<Log>>,
    #[cfg(windows)]
    _job: Option<job::Job>,
}

#[derive(Default)]
struct Log {
    lines: VecDeque<String>,
    url: Option<String>,
}

impl Drop for Running {
    fn drop(&mut self) {
        // The job closes after this and takes the rest of the tree with it.
        let _ = self.child.start_kill();
    }
}

/// The answer to a DevTools Protocol call on the pane's page.
pub type Reply = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>;

/// The app's browser pane. The desktop host installs one at startup; the headless engine has
/// none, so previews are only reported and the page tools are unavailable.
pub trait PaneHost: Send + Sync {
    /// Shows `url` in the pane.
    fn open(&self, directory: &Path, url: &str);
    /// A DevTools Protocol call on the pane's own page, never on Cryzo's interface.
    fn cdp(&self, method: &str, params: Value) -> Reply;
}

impl Previews {
    /// Stops every preview, when the app quits.
    pub fn stop_all(&self) {
        self.0.lock().unwrap().clear();
    }

    fn stop(&self, root: &Path) -> bool {
        self.0.lock().unwrap().remove(root).is_some()
    }
}

/// Starts (or restarts) the project's dev server and waits until it answers.
pub async fn start(engine: &Engine, root: &Path, program: &str, args: &[String]) -> Result<String> {
    engine.previews.stop(root);
    let mut command = crate::tools::command(program, args, root)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    #[cfg(windows)]
    let job = child
        .raw_handle()
        .and_then(|handle| job::Job::holding(handle).ok());
    let log = Arc::new(Mutex::new(Log::default()));
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(read(stdout, log.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(read(stderr, log.clone()));
    }
    engine.previews.0.lock().unwrap().insert(
        root.to_path_buf(),
        Running {
            child,
            log: log.clone(),
            #[cfg(windows)]
            _job: job,
        },
    );
    let deadline = tokio::time::Instant::now() + READY_WITHIN;
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let exited = {
            let mut previews = engine.previews.0.lock().unwrap();
            let Some(running) = previews.get_mut(root) else {
                bail!("the preview was stopped before it started");
            };
            running.child.try_wait()?.map(|status| status.code())
        };
        if let Some(code) = exited {
            engine.previews.stop(root);
            bail!(
                "the dev server exited ({}) before it was ready. Last output:\n{}",
                code.map_or("no exit code".to_owned(), |code| format!("code {code}")),
                tail(&log, 60)
            );
        }
        let url = log.lock().unwrap().url.clone();
        if let Some(url) = url
            && answers(&url).await
        {
            if let Some(pane) = engine.pane.get() {
                pane.open(root, &url);
            }
            return Ok(format!(
                "The app is running at {url} and is shown in Cryzo's browser pane.\nRecent output:\n{}",
                tail(&log, 15)
            ));
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "the dev server didn't print a local address within {} seconds. It is still running; check preview_logs. Last output:\n{}",
                READY_WITHIN.as_secs(),
                tail(&log, 40)
            );
        }
    }
}

/// The address and recent output of the project's preview.
pub fn logs(engine: &Engine, root: &Path, lines: usize) -> Result<String> {
    let previews = engine.previews.0.lock().unwrap();
    let running = previews
        .get(root)
        .context("no preview is running for this project; start one with preview")?;
    let url = running.log.lock().unwrap().url.clone();
    Ok(format!(
        "Address: {}\nRecent output:\n{}",
        url.as_deref().unwrap_or("not printed yet"),
        tail(&running.log, lines.clamp(1, KEPT_LINES))
    ))
}

pub fn stop(engine: &Engine, root: &Path) -> String {
    if engine.previews.stop(root) {
        "The preview was stopped.".to_owned()
    } else {
        "No preview was running.".to_owned()
    }
}

/// The page tools act only on the project's own running preview: the agent tests the app it is
/// building, and can't reach whatever else the user opened in the pane.
async fn preview_page(engine: &Engine, root: &Path) -> Result<Arc<dyn PaneHost>> {
    let pane = engine
        .pane
        .get()
        .cloned()
        .context("Cryzo's browser pane is only available in the desktop app")?;
    let url = engine
        .previews
        .0
        .lock()
        .unwrap()
        .get(root)
        .and_then(|running| running.log.lock().unwrap().url.clone())
        .context("no preview is running for this project; start one with preview")?;
    let origin = evaluate(&*pane, "location.origin").await?;
    ensure!(
        origin.as_str() == Some(url.as_str()),
        "the browser pane shows another page; reopen the preview at {url} first"
    );
    Ok(pane)
}

async fn evaluate(pane: &dyn PaneHost, expression: &str) -> Result<Value> {
    let reply = pane
        .cdp(
            "Runtime.evaluate",
            json!({"expression":expression,"returnByValue":true,"awaitPromise":true}),
        )
        .await?;
    if let Some(error) = reply.get("exceptionDetails") {
        bail!(
            "the page threw: {}",
            error["exception"]["description"]
                .as_str()
                .or(error["text"].as_str())
                .unwrap_or("an error")
        );
    }
    Ok(reply["result"]["value"].clone())
}

/// Numbers the visible controls (as `data-cryzo-n`) and returns the page's text and that list.
const READ: &str = r#"(() => {
  const shown = (el) => { const r = el.getBoundingClientRect(); const s = getComputedStyle(el); return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none" };
  document.querySelectorAll("[data-cryzo-n]").forEach((el) => el.removeAttribute("data-cryzo-n"));
  const elements = [];
  for (const el of document.querySelectorAll("a[href],button,input,textarea,select,summary,[role=button],[role=link],[role=checkbox],[role=tab],[contenteditable=true]")) {
    if (elements.length >= 150 || !shown(el)) continue;
    const n = elements.length + 1;
    el.setAttribute("data-cryzo-n", String(n));
    const label = (el.getAttribute("aria-label") || el.innerText || el.value || el.placeholder || el.title || "").trim().replace(/\s+/g, " ").slice(0, 80);
    elements.push("[" + n + "] " + el.tagName.toLowerCase() + (el.type ? "(" + el.type + ")" : "") + " " + label);
  }
  const text = (document.body ? document.body.innerText : "").replace(/\n{3,}/g, "\n\n").slice(0, 6000);
  return JSON.stringify({ title: document.title, url: location.href, text, elements });
})()"#;

/// The preview's text and its numbered controls, for page_act.
pub async fn page_read(engine: &Engine, root: &Path) -> Result<String> {
    let pane = preview_page(engine, root).await?;
    let page: Value = serde_json::from_str(evaluate(&*pane, READ).await?.as_str().unwrap_or("{}"))?;
    let elements: Vec<&str> = page["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    Ok(format!(
        "{} ({})\n\nControls (use the number with page_act):\n{}\n\nText:\n{}",
        page["title"].as_str().unwrap_or_default(),
        page["url"].as_str().unwrap_or_default(),
        elements.join("\n"),
        page["text"].as_str().unwrap_or_default()
    ))
}

/// Clicks, types into or presses a key on control `element` from page_read, with real input
/// events, as a user would.
pub async fn page_act(
    engine: &Engine,
    root: &Path,
    element: u32,
    action: &str,
    text: Option<&str>,
) -> Result<String> {
    let pane = preview_page(engine, root).await?;
    let found = evaluate(
        &*pane,
        &format!(
            r#"(() => {{ const el = document.querySelector('[data-cryzo-n="{element}"]'); if (!el) return null; el.scrollIntoView({{ block: "center" }}); if ({focus}) {{ el.focus(); if (el.select) el.select(); }} const r = el.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; }})()"#,
            focus = action != "click"
        ),
    )
    .await?;
    let Some([x, y]) = found
        .as_array()
        .and_then(|point| Some([point.first()?.as_f64()?, point.get(1)?.as_f64()?]))
    else {
        bail!("control {element} isn't on the page any more; call page_read again");
    };
    match action {
        "click" => {
            for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
                pane.cdp(
                    "Input.dispatchMouseEvent",
                    json!({"type":kind,"x":x,"y":y,"button":"left","clickCount":1}),
                )
                .await?;
            }
        }
        "type" => {
            pane.cdp(
                "Input.insertText",
                json!({"text":text.context("type needs text")?}),
            )
            .await?;
        }
        "press" => {
            let key = text.unwrap_or("Enter");
            let (virtual_key, typed) = match key {
                "Enter" => (13, "\r"),
                "Tab" => (9, ""),
                "Escape" => (27, ""),
                "Backspace" => (8, ""),
                _ => bail!("press supports Enter, Tab, Escape and Backspace"),
            };
            for kind in ["keyDown", "keyUp"] {
                let typed = if kind == "keyDown" { typed } else { "" };
                pane.cdp(
                    "Input.dispatchKeyEvent",
                    json!({"type":kind,"key":key,"code":key,"windowsVirtualKeyCode":virtual_key,"text":typed}),
                )
                .await?;
            }
        }
        _ => bail!("action must be click, type or press"),
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    let url = evaluate(&*pane, "location.href").await?;
    Ok(format!(
        "Done. The page is at {}. Call page_read to see the result.",
        url.as_str().unwrap_or_default()
    ))
}

/// The preview as the user sees it, as tool content: a line of text and the picture as a
/// `Tool.FileContent` data URL. JPEG keeps it small; the model reads the page with page_read first.
pub async fn page_screenshot(engine: &Engine, root: &Path) -> Result<Value> {
    let pane = preview_page(engine, root).await?;
    let address = evaluate(&*pane, "location.href").await?;
    let shot = pane
        .cdp(
            "Page.captureScreenshot",
            json!({"format":"jpeg","quality":70}),
        )
        .await?;
    let data = shot["data"]
        .as_str()
        .context("the browser pane returned no screenshot")?;
    ensure!(data.len() <= 3_000_000, "the screenshot is too large");
    Ok(json!([
        {"type":"text","text":format!("Screenshot of {} as it shows in Cryzo's browser pane.", address.as_str().unwrap_or_default())},
        {"type":"file","uri":format!("data:image/jpeg;base64,{data}"),"mime":"image/jpeg","name":"page.jpg"},
    ]))
}

/// Errors the preview's page reported (uncaught errors, rejected promises, console.error),
/// collected by the pane's page script since the page loaded.
pub async fn page_errors(engine: &Engine, root: &Path) -> Result<String> {
    let pane = preview_page(engine, root).await?;
    let errors = evaluate(&*pane, "JSON.stringify(window.__cryzoErrors || [])").await?;
    let errors: Vec<String> = serde_json::from_str(errors.as_str().unwrap_or("[]"))?;
    Ok(if errors.is_empty() {
        "No errors since the page loaded.".to_owned()
    } else {
        errors.join("\n")
    })
}

async fn read(stream: impl AsyncRead + Unpin, log: Arc<Mutex<Log>>) {
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = plain(&line);
        let mut log = log.lock().unwrap();
        if log.url.is_none() {
            log.url = local_url(&line);
        }
        log.lines.push_back(line);
        if log.lines.len() > KEPT_LINES {
            log.lines.pop_front();
        }
    }
}

fn tail(log: &Mutex<Log>, count: usize) -> String {
    let log = log.lock().unwrap();
    let skip = log.lines.len().saturating_sub(count);
    log.lines
        .iter()
        .skip(skip)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first local address in a line of dev server output, like Vite's
/// `Local:   http://localhost:5173/`.
fn local_url(line: &str) -> Option<String> {
    ["http://localhost:", "http://127.0.0.1:", "http://[::1]:"]
        .iter()
        .filter_map(|prefix| line.find(prefix))
        .min()
        .map(|start| {
            let url: String = line[start..]
                .chars()
                .take_while(|char| !char.is_whitespace() && *char != ',' && *char != ')')
                .collect();
            url.trim_end_matches('/').to_owned()
        })
        .filter(|url| {
            url.rsplit(':')
                .next()
                .is_some_and(|port| port.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
}

/// Whether something accepts connections at the URL's port on this computer.
async fn answers(url: &str) -> bool {
    let Some(port) = url
        .rsplit(':')
        .next()
        .and_then(|rest| rest.split('/').next())
        .and_then(|port| port.parse::<u16>().ok())
    else {
        return false;
    };
    for host in ["127.0.0.1", "[::1]"] {
        let connect = tokio::net::TcpStream::connect(format!("{host}:{port}"));
        if let Ok(Ok(_)) = tokio::time::timeout(Duration::from_millis(500), connect).await {
            return true;
        }
    }
    false
}

/// Output without terminal colour codes.
fn plain(line: &str) -> String {
    let mut text = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(char) = chars.next() {
        if char == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for code in chars.by_ref() {
                    if code.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        text.push(char);
    }
    text
}

#[cfg(windows)]
mod job {
    use std::os::windows::io::RawHandle;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    /// A job object that ends every process in it when it closes.
    pub struct Job(HANDLE);

    // The handle is only closed, from whichever thread drops it.
    unsafe impl Send for Job {}

    impl Job {
        pub fn holding(process: RawHandle) -> windows::core::Result<Self> {
            // SAFETY: plain Win32 calls on a new job handle and a live process handle owned by
            // the tokio child; the job handle is closed in Drop.
            unsafe {
                let job = Job(CreateJobObjectW(None, None)?);
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )?;
                AssignProcessToJobObject(job.0, HANDLE(process))?;
                Ok(job)
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateJobObjectW and is closed once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_server_addresses_are_found_in_colored_output() {
        let line = plain(
            "  \u{1b}[32m➜\u{1b}[39m  \u{1b}[1mLocal\u{1b}[22m:   \u{1b}[36mhttp://localhost:\u{1b}[1m5173\u{1b}[22m/\u{1b}[39m",
        );
        assert_eq!(local_url(&line).as_deref(), Some("http://localhost:5173"));
        assert_eq!(
            local_url("ready on http://127.0.0.1:8787, press b").as_deref(),
            Some("http://127.0.0.1:8787")
        );
        assert_eq!(local_url("see https://vite.dev for docs"), None);
        assert_eq!(local_url("http://localhost:abc"), None);
    }

    #[tokio::test]
    async fn a_port_that_accepts_connections_answers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(answers(&format!("http://localhost:{port}")).await);
        drop(listener);
        assert!(!answers(&format!("http://localhost:{port}")).await);
    }
}
