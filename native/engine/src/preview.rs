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
        // On Unix the server leads its own process group, so npm, node and the bundler it started
        // stop with it.
        #[cfg(unix)]
        if let Some(group) = self.child.id() {
            let _ = std::process::Command::new("kill")
                .args(["-KILL", &format!("-{group}")])
                .status();
        }
        // On Windows the job closes after this and takes the rest of the tree with it.
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
    #[cfg(unix)]
    command.process_group(0);
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

/// Lines a page_read shows, as many as a desktop snapshot by default.
const PAGE_LINES: usize = 250;

/// The preview as its accessibility tree, the way browser agents read pages: one line per element
/// that matters (`[e12] button "Sign in" focused`), with IDs page_act takes. Cryzo's Chrome tools
/// read pages through the same code.
pub async fn page_read(engine: &Engine, root: &Path) -> Result<String> {
    let pane = preview_page(engine, root).await?;
    let tree = pane.cdp("Accessibility.getFullAXTree", json!({})).await?;
    let (mut lines, _) = cryzo_desktop_control::ax::snapshot(
        &tree,
        &mut |backend| format!("e{backend}"),
        PAGE_LINES,
    )?;
    if lines.len() >= PAGE_LINES {
        lines.push("  … cut off at the line budget; scroll with page_act to read further".into());
    }
    let page = evaluate(&*pane, "[document.title, location.href]").await?;
    Ok(format!(
        "{} ({})\n{}",
        page[0].as_str().unwrap_or_default(),
        page[1].as_str().unwrap_or_default(),
        lines.join("\n")
    ))
}

/// Acts on element `element` (an ID from page_read) with real input events, as a user would:
/// click it, type text into it, choose an option of a select, press a key on it, or scroll to it.
pub async fn page_act(
    engine: &Engine,
    root: &Path,
    element: &str,
    action: &str,
    text: Option<&str>,
) -> Result<String> {
    let pane = preview_page(engine, root).await?;
    let backend = element
        .trim()
        .trim_matches(['[', ']'])
        .strip_prefix('e')
        .and_then(|id| id.parse::<i64>().ok())
        .context("element must be an ID from page_read, like e12")?;
    let node = json!({"backendNodeId":backend});
    pane.cdp("DOM.enable", json!({})).await?;
    let object = pane
        .cdp("DOM.resolveNode", node.clone())
        .await
        .ok()
        .and_then(|reply| reply["object"]["objectId"].as_str().map(str::to_owned))
        .with_context(|| format!("{element} isn't on the page any more; call page_read again"))?;
    let call = |function: &'static str, arguments: Value| {
        pane.cdp(
            "Runtime.callFunctionOn",
            json!({"objectId":object,"functionDeclaration":function,"arguments":arguments,"returnByValue":true}),
        )
    };
    let _ = pane.cdp("DOM.scrollIntoViewIfNeeded", node.clone()).await;
    match action {
        "click" => match center(&*pane, &node).await {
            Some((x, y)) => {
                for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
                    pane.cdp(
                        "Input.dispatchMouseEvent",
                        json!({"type":kind,"x":x,"y":y,"button":"left","clickCount":1}),
                    )
                    .await?;
                }
            }
            // Elements without a box (zero-sized, behind a menu) still take a DOM click.
            None => {
                call("function(){ this.click() }", json!([])).await?;
            }
        },
        "type" => {
            let text = text.context("type needs text")?;
            // Clear the field so the text replaces it, then insert it the way typing would, so
            // page frameworks see normal input events.
            call(
                "function(){ this.focus(); if ('value' in this) { this.value = ''; this.dispatchEvent(new Event('input', {bubbles: true})) } else if (this.isContentEditable) { this.textContent = '' } }",
                json!([]),
            )
            .await?;
            pane.cdp("DOM.focus", node).await?;
            pane.cdp("Input.insertText", json!({"text":text})).await?;
        }
        "select" => {
            let text = text.context("select needs the option's text")?;
            let picked = call(
                "function(v){ const o = [...(this.options || [])].find(o => o.text.trim() === v || o.value === v); if (!o) return false; this.value = o.value; this.dispatchEvent(new Event('input', {bubbles: true})); this.dispatchEvent(new Event('change', {bubbles: true})); return true }",
                json!([{"value":text}]),
            )
            .await?;
            ensure!(
                picked["result"]["value"] == json!(true),
                "no option {text:?} in {element}"
            );
        }
        "press" => {
            let key = text.unwrap_or("Enter");
            let (virtual_key, typed) = match key {
                "Enter" => (13, "\r"),
                "Tab" => (9, ""),
                "Escape" => (27, ""),
                "Backspace" => (8, ""),
                "ArrowDown" => (40, ""),
                "ArrowUp" => (38, ""),
                _ => bail!("press supports Enter, Tab, Escape, Backspace, ArrowDown and ArrowUp"),
            };
            pane.cdp("DOM.focus", node).await?;
            for kind in ["keyDown", "keyUp"] {
                let typed = if kind == "keyDown" { typed } else { "" };
                pane.cdp(
                    "Input.dispatchKeyEvent",
                    json!({"type":kind,"key":key,"code":key,"windowsVirtualKeyCode":virtual_key,"text":typed}),
                )
                .await?;
            }
        }
        "scroll_into_view" => {}
        _ => bail!("action must be click, type, select, press or scroll_into_view"),
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    let url = evaluate(&*pane, "location.href").await?;
    Ok(format!(
        "Done. The page is at {}. Call page_read to see the result.",
        url.as_str().unwrap_or_default()
    ))
}

/// The middle of an element's content box in the page's CSS pixels, where a click lands.
async fn center(pane: &dyn PaneHost, node: &Value) -> Option<(f64, f64)> {
    let model = pane.cdp("DOM.getBoxModel", node.clone()).await.ok()?;
    let quad = model["model"]["content"]
        .as_array()?
        .iter()
        .filter_map(Value::as_f64)
        .collect::<Vec<_>>();
    (quad.len() == 8).then(|| {
        (
            (quad[0] + quad[2] + quad[4] + quad[6]) / 4.0,
            (quad[1] + quad[3] + quad[5] + quad[7]) / 4.0,
        )
    })
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
pub(crate) use job::Job;

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
