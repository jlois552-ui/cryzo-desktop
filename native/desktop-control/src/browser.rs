//! A Chrome window Cryzo drives through the Chrome DevTools Protocol, with no extension.
//!
//! Chrome runs with its own profile under `%LOCALAPPDATA%\Cryzo\desktop-control\browser`, because
//! Chrome refuses remote debugging on the user's default profile. The user signs in to sites there
//! once. Pages are read through Chrome's accessibility tree as the same compact lines desktop
//! snapshots use (`[b12] link "Download for Windows"`), and elements are acted on by ID or name.
//! The debugging port listens on localhost only, at a random port Chrome picks.

use std::{
    collections::{HashMap, HashSet},
    net::TcpStream,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use crate::uia::{changes, stopped};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const LOAD_LIMIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(150);
const QUIET: Duration = Duration::from_millis(400);

/// Roles that are worth a line even without a name, because the agent can act on them.
const INTERACTIVE: [&str; 15] = [
    "link",
    "button",
    "textbox",
    "searchbox",
    "combobox",
    "checkbox",
    "radio",
    "menuitem",
    "tab",
    "option",
    "switch",
    "slider",
    "listbox",
    "spinbutton",
    "menuitemcheckbox",
];

/// Layout roles that never get a line of their own; their children still print.
const LAYOUT: [&str; 8] = [
    "generic",
    "none",
    "presentation",
    "InlineTextBox",
    "LineBreak",
    "RootWebArea",
    "paragraph",
    "group",
];

pub struct Browser {
    socket: WebSocket<TcpStream>,
    /// Flattened CDP session of the tab being driven.
    session: String,
    target: String,
    /// Page targets that existed after the last action, to notice tabs a click opened.
    known: HashSet<String>,
    next: u64,
    /// DOM backend node ID → short ID, so an element keeps its ID while the page lives.
    ids: HashMap<i64, String>,
    counter: usize,
    nodes: HashMap<String, Node>,
    last: Option<Vec<String>>,
    downloads: Vec<Download>,
    /// Notes for the next result, such as a tab that opened.
    notes: Vec<String>,
}

#[derive(Clone)]
struct Node {
    backend: i64,
    role: String,
    name: String,
}

struct Download {
    guid: String,
    file: String,
    /// Where Chrome saved it, which differs from `file` when that name was taken.
    path: Option<String>,
    /// Whether a result already said it finished.
    reported: bool,
    state: String,
    received: u64,
    total: u64,
}

impl Browser {
    /// Connects to the Cryzo Chrome, starting it when it is not running.
    pub fn connect() -> Result<Self> {
        let profile = profile()?;
        let port_file = profile.join("DevToolsActivePort");
        if let Ok(browser) = Self::attach(&port_file) {
            return Ok(browser);
        }
        let _ = std::fs::remove_file(&port_file);
        Command::new(chrome()?)
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "about:blank",
            ])
            // Chrome outlives this process; inheriting the MCP server's stdout would corrupt its
            // JSON-RPC stream and keep the pipe open after the server exits.
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("starting Chrome")?;
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            std::thread::sleep(Duration::from_millis(200));
            match Self::attach(&port_file) {
                Ok(browser) => return Ok(browser),
                Err(error) if Instant::now() >= deadline => {
                    return Err(error.context("Chrome did not start its debugging port"));
                }
                Err(_) => {}
            }
        }
    }

    fn attach(port_file: &std::path::Path) -> Result<Self> {
        let text = std::fs::read_to_string(port_file)?;
        let mut lines = text.lines();
        let port: u16 = lines.next().context("no port")?.trim().parse()?;
        let path = lines.next().context("no browser path")?.trim();
        let stream = TcpStream::connect(("127.0.0.1", port))?;
        stream.set_read_timeout(Some(COMMAND_TIMEOUT))?;
        let (socket, _) = tungstenite::client(format!("ws://127.0.0.1:{port}{path}"), stream)
            .map_err(|error| anyhow!("connecting to Chrome: {error}"))?;
        let mut browser = Self {
            socket,
            session: String::new(),
            target: String::new(),
            known: HashSet::new(),
            next: 0,
            ids: HashMap::new(),
            counter: 0,
            nodes: HashMap::new(),
            last: None,
            downloads: Vec::new(),
            notes: Vec::new(),
        };
        let downloads = downloads_dir()?;
        browser.command(
            None,
            "Browser.setDownloadBehavior",
            json!({"behavior":"allow","downloadPath":downloads,"eventsEnabled":true}),
        )?;
        let pages = browser.pages()?;
        let target =
            match pages.first() {
                Some(target) => target.clone(),
                None => browser.command(
                    None,
                    "Target.createTarget",
                    json!({"url":"about:blank"}),
                )?["targetId"]
                    .as_str()
                    .context("no target")?
                    .to_string(),
            };
        browser.known = pages.into_iter().collect();
        browser.switch(&target)?;
        Ok(browser)
    }

    /// Whether the connection still answers, so a closed Chrome gets restarted.
    pub fn alive(&mut self) -> bool {
        self.command(None, "Browser.getVersion", json!({})).is_ok()
    }

    pub fn call(
        &mut self,
        command: &str,
        args: &Value,
        max_lines: usize,
        passwords: bool,
    ) -> Result<String> {
        let text = |key: &str| args.get(key).and_then(Value::as_str);
        let required = |key: &str| text(key).ok_or_else(|| anyhow!("{command} needs {key}"));
        match command {
            "browser_open" => {
                stopped()?;
                self.open(required("url")?)?;
                let lines = self.settle(max_lines)?;
                let header = self.header()?;
                self.last = Some(lines.clone());
                Ok(format!("{header}\n{}{}", lines.join("\n"), self.notes()))
            }
            "browser_snapshot" => {
                self.pump(Duration::ZERO);
                let lines = self.read(max_lines)?;
                self.last = Some(lines.clone());
                let header = self.header()?;
                Ok(format!("{header}\n{}{}", lines.join("\n"), self.notes()))
            }
            "browser_find" => self.find(required("text")?, text("role")),
            "browser_act" => {
                stopped()?;
                let id = match text("id") {
                    Some(id) => id.to_string(),
                    None => self.resolve(
                        text("name").ok_or_else(|| anyhow!("browser_act needs id or name"))?,
                        text("role"),
                    )?,
                };
                self.act(&id, required("action")?, text("value"), passwords)?;
                self.after(max_lines)
            }
            "browser_key" => {
                stopped()?;
                self.key(required("keys")?)?;
                self.after(max_lines)
            }
            "browser_downloads" => {
                self.wait_downloads(args.get("wait").and_then(Value::as_bool).unwrap_or(false))
            }
            _ => bail!("Unknown browser tool {command:?}"),
        }
    }

    /// The host of the page or URL a command acts on, for approvals and "ask once per app".
    pub fn site(&mut self, command: &str, args: &Value) -> Option<String> {
        let url = match command {
            "browser_open" => args.get("url")?.as_str()?.to_string(),
            _ => self.url().ok()?,
        };
        Some(host(&url).to_string()).filter(|host| !host.is_empty())
    }

    /// A human sentence for an approval card.
    pub fn describe(&mut self, command: &str, args: &Value) -> String {
        let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
        let site = self
            .site(command, args)
            .unwrap_or_else(|| "the page".into());
        match command {
            "browser_open" => format!("open {} in the Cryzo browser", text("url")),
            "browser_key" => format!("press {:?} on {site}", text("keys")),
            "browser_act" => {
                let (role, name) = match self.nodes.get(text("id")) {
                    Some(node) => (node.role.clone(), node.name.clone()),
                    None => (
                        args.get("role")
                            .and_then(Value::as_str)
                            .unwrap_or("element")
                            .to_string(),
                        text("name").to_string(),
                    ),
                };
                let typed = match text("action") {
                    "type" | "select" => format!(": {:?}", text("value")),
                    _ => String::new(),
                };
                format!("{} {role} {name:?} on {site}{typed}", text("action"))
            }
            _ => command.to_string(),
        }
    }

    fn open(&mut self, url: &str) -> Result<()> {
        match url {
            "back" | "forward" => {
                let history = self.send("Page.getNavigationHistory", json!({}))?;
                let index = history["currentIndex"].as_i64().unwrap_or_default()
                    + if url == "back" { -1 } else { 1 };
                let entry = history["entries"]
                    .get(
                        usize::try_from(index)
                            .ok()
                            .context("no page to go back to")?,
                    )
                    .context("no page in that direction")?;
                self.send(
                    "Page.navigateToHistoryEntry",
                    json!({"entryId":entry["id"]}),
                )?;
            }
            "reload" => {
                self.send("Page.reload", json!({}))?;
            }
            _ => {
                let url = if url.contains("://") || url.starts_with("about:") {
                    url.to_string()
                } else {
                    format!("https://{url}")
                };
                let result = self.send("Page.navigate", json!({"url":url}))?;
                if let Some(error) = result["errorText"].as_str() {
                    bail!("could not open {url}: {error}");
                }
            }
        }
        Ok(())
    }

    fn act(&mut self, id: &str, action: &str, value: Option<&str>, passwords: bool) -> Result<()> {
        if !self.nodes.contains_key(id) {
            self.read(usize::MAX)?;
        }
        let node = self
            .nodes
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("Unknown element {id}; take a new browser_snapshot"))?;
        let backend = json!({"backendNodeId":node.backend});
        match action {
            "click" | "double_click" => {
                let _ = self.send("DOM.scrollIntoViewIfNeeded", backend.clone());
                match self.center(node.backend) {
                    Ok((x, y)) => {
                        let count = if action == "double_click" { 2 } else { 1 };
                        self.send(
                            "Input.dispatchMouseEvent",
                            json!({"type":"mouseMoved","x":x,"y":y}),
                        )?;
                        for kind in ["mousePressed", "mouseReleased"] {
                            self.send(
                                "Input.dispatchMouseEvent",
                                json!({"type":kind,"x":x,"y":y,"button":"left","clickCount":count}),
                            )?;
                        }
                    }
                    // Elements without a box (hidden behind a menu, zero-sized) still take a DOM click.
                    Err(_) => {
                        self.call_on(node.backend, "function(){ this.click() }", json!([]))?;
                    }
                }
            }
            "type" => {
                let value = value.context("type needs value")?;
                let password = self.call_on(
                    node.backend,
                    "function(){ return this.type === 'password' }",
                    json!([]),
                )?;
                ensure!(
                    passwords || password != json!(true),
                    "typing into password fields is turned off in desktop-control settings"
                );
                // Clear the field so the text replaces it, then insert it the way typing would, so
                // page frameworks see normal input events.
                self.call_on(
                    node.backend,
                    "function(){ this.focus(); if ('value' in this) { this.value = ''; this.dispatchEvent(new Event('input', {bubbles: true})) } else if (this.isContentEditable) { this.textContent = '' } }",
                    json!([]),
                )?;
                self.send("DOM.focus", backend)?;
                self.send("Input.insertText", json!({"text":value}))?;
            }
            "select" => {
                let value = value.context("select needs value")?;
                let picked = self.call_on(
                    node.backend,
                    "function(v){ const o = [...(this.options || [])].find(o => o.text.trim() === v || o.value === v); if (!o) return false; this.value = o.value; this.dispatchEvent(new Event('input', {bubbles: true})); this.dispatchEvent(new Event('change', {bubbles: true})); return true }",
                    json!([{"value":value}]),
                )?;
                ensure!(picked == json!(true), "no option {value:?} in {id}");
            }
            "focus" => {
                self.send("DOM.focus", backend)?;
            }
            "scroll_into_view" => {
                self.send("DOM.scrollIntoViewIfNeeded", backend)?;
            }
            _ => bail!("Unknown browser action {action:?}"),
        }
        Ok(())
    }

    fn key(&mut self, keys: &str) -> Result<()> {
        let name = keys
            .trim()
            .trim_start_matches('{')
            .trim_end_matches('}')
            .to_lowercase();
        let (key, code) = match name.as_str() {
            "enter" => ("Enter", 13),
            "tab" => ("Tab", 9),
            "escape" | "esc" => ("Escape", 27),
            "backspace" => ("Backspace", 8),
            "space" => (" ", 32),
            "pagedown" => ("PageDown", 34),
            "pageup" => ("PageUp", 33),
            "home" => ("Home", 36),
            "end" => ("End", 35),
            "down" | "arrowdown" => ("ArrowDown", 40),
            "up" | "arrowup" => ("ArrowUp", 38),
            "left" | "arrowleft" => ("ArrowLeft", 37),
            "right" | "arrowright" => ("ArrowRight", 39),
            _ => bail!(
                "browser_key takes one key: enter, tab, escape, backspace, space, pageup, pagedown, home, end or an arrow; use browser_act type for text"
            ),
        };
        let text = match key {
            "Enter" => "\r",
            " " => " ",
            _ => "",
        };
        self.send(
            "Input.dispatchKeyEvent",
            json!({"type":if text.is_empty() {"rawKeyDown"} else {"keyDown"},"key":key,"code":key,"windowsVirtualKeyCode":code,"text":text}),
        )?;
        self.send(
            "Input.dispatchKeyEvent",
            json!({"type":"keyUp","key":key,"code":key,"windowsVirtualKeyCode":code}),
        )?;
        Ok(())
    }

    fn find(&mut self, text: &str, role: Option<&str>) -> Result<String> {
        self.read(usize::MAX)?;
        let wanted = text.to_lowercase();
        let lines = self
            .nodes
            .iter()
            .filter(|(_, node)| {
                node.name.to_lowercase().contains(&wanted)
                    && role.is_none_or(|role| node.role.eq_ignore_ascii_case(role))
            })
            .map(|(id, node)| {
                (
                    id.trim_start_matches('b')
                        .parse::<usize>()
                        .unwrap_or_default(),
                    format!("  [{id}] {} {}", node.role, quote(&node.name)),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let header = self.header()?;
        if lines.is_empty() {
            return Ok(format!("No element matching {text:?} on {header}"));
        }
        Ok(format!(
            "{header}\n{}",
            lines.into_values().take(50).collect::<Vec<_>>().join("\n")
        ))
    }

    fn resolve(&mut self, name: &str, role: Option<&str>) -> Result<String> {
        self.read(usize::MAX)?;
        let wanted = name.to_lowercase();
        let matches = |exact: bool| {
            let mut found = self
                .nodes
                .iter()
                .filter(|(_, node)| {
                    let have = node.name.to_lowercase();
                    (if exact {
                        have == wanted
                    } else {
                        have.contains(&wanted)
                    }) && role.is_none_or(|role| node.role.eq_ignore_ascii_case(role))
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            // Earlier IDs are earlier in the page.
            found.sort_by_key(|id| {
                id.trim_start_matches('b')
                    .parse::<usize>()
                    .unwrap_or_default()
            });
            found.into_iter().next()
        };
        // Many search boxes have no label; when the page has exactly one element of the asked
        // role, that is the one meant.
        let only = || {
            let role = role?;
            let mut found = self
                .nodes
                .iter()
                .filter(|(_, node)| node.role.eq_ignore_ascii_case(role));
            let first = found.next()?;
            found.next().is_none().then(|| first.0.clone())
        };
        matches(true)
            .or_else(|| matches(false))
            .or_else(only)
            .ok_or_else(|| anyhow!("No element matching {name:?} on this page; try browser_find"))
    }

    /// What changed once the page settles, plus a tab the action opened and downloads it started.
    fn after(&mut self, max_lines: usize) -> Result<String> {
        let lines = self.settle(max_lines)?;
        let header = self.header()?;
        let result = match self.last.replace(lines.clone()) {
            Some(previous) => changes(&header, &previous, &lines),
            None => format!("{header}\n{}", lines.join("\n")),
        };
        Ok(format!("{result}{}", self.notes()))
    }

    /// Waits for the page to load and its accessibility tree to stop changing.
    fn settle(&mut self, max_lines: usize) -> Result<Vec<String>> {
        let deadline = Instant::now() + LOAD_LIMIT;
        std::thread::sleep(POLL);
        self.pump(Duration::ZERO);
        self.follow_new_tab()?;
        while Instant::now() < deadline {
            let state = self.send(
                "Runtime.evaluate",
                json!({"expression":"document.readyState","returnByValue":true}),
            );
            if state.is_ok_and(|state| state["result"]["value"] == "complete") {
                break;
            }
            std::thread::sleep(POLL);
        }
        let mut lines = self.read(max_lines).unwrap_or_default();
        let mut quiet = Instant::now();
        while quiet.elapsed() < QUIET && Instant::now() < deadline {
            std::thread::sleep(POLL);
            self.pump(Duration::ZERO);
            let next = self.read(max_lines).unwrap_or_default();
            if next != lines {
                lines = next;
                quiet = Instant::now();
            }
        }
        Ok(lines)
    }

    /// Switches to a tab that the last action opened, like a link with target=_blank.
    fn follow_new_tab(&mut self) -> Result<()> {
        let pages = self.pages()?;
        let opened = pages
            .iter()
            .find(|page| !self.known.contains(*page))
            .cloned();
        self.known = pages.into_iter().collect();
        if let Some(page) = opened {
            self.switch(&page)?;
            self.notes.push("opened a new tab, now driving it".into());
        }
        Ok(())
    }

    fn switch(&mut self, target: &str) -> Result<()> {
        let attached = self.command(
            None,
            "Target.attachToTarget",
            json!({"targetId":target,"flatten":true}),
        )?;
        self.session = attached["sessionId"]
            .as_str()
            .context("no session")?
            .to_string();
        self.target = target.to_string();
        self.send("Page.enable", json!({}))?;
        self.send("DOM.enable", json!({}))?;
        let _ = self.command(None, "Target.activateTarget", json!({"targetId":target}));
        self.ids.clear();
        self.nodes.clear();
        self.last = None;
        Ok(())
    }

    fn pages(&mut self) -> Result<Vec<String>> {
        Ok(
            self.command(None, "Target.getTargets", json!({}))?["targetInfos"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|info| info["type"] == "page")
                .filter_map(|info| info["targetId"].as_str().map(str::to_string))
                .collect(),
        )
    }

    fn url(&mut self) -> Result<String> {
        let info = self.command(
            None,
            "Target.getTargetInfo",
            json!({"targetId":self.target}),
        )?;
        Ok(info["targetInfo"]["url"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    fn header(&mut self) -> Result<String> {
        let info = self.command(
            None,
            "Target.getTargetInfo",
            json!({"targetId":self.target}),
        )?;
        Ok(format!(
            "browser tab {} {}",
            quote(info["targetInfo"]["title"].as_str().unwrap_or_default()),
            info["targetInfo"]["url"].as_str().unwrap_or_default()
        ))
    }

    /// Lines after a result: a tab the action opened, downloads in progress, and each finished
    /// download once.
    fn notes(&mut self) -> String {
        let notes = self
            .notes
            .drain(..)
            .map(|note| format!("({note})"))
            .chain(
                self.downloads
                    .iter_mut()
                    .filter(|download| !download.reported)
                    .map(|download| {
                        download.reported = download.state != "inProgress";
                        download.line()
                    }),
            )
            .collect::<Vec<_>>();
        notes.iter().map(|note| format!("\n{note}")).collect()
    }

    /// The page as indented lines, from Chrome's accessibility tree.
    fn read(&mut self, max_lines: usize) -> Result<Vec<String>> {
        let tree = self.send("Accessibility.getFullAXTree", json!({}))?;
        let nodes = tree["nodes"].as_array().context("no accessibility tree")?;
        let by_id = nodes
            .iter()
            .filter_map(|node| Some((node["nodeId"].as_str()?, node)))
            .collect::<HashMap<_, _>>();
        let root = nodes.first().context("empty page")?;
        let mut lines = Vec::new();
        let mut seen = HashMap::new();
        self.visit(root, &by_id, 0, "", &mut lines, &mut seen, max_lines);
        // Only the current page's elements, so find never offers one from a page already left.
        self.nodes = seen;
        if lines.len() >= max_lines {
            lines.push("  … cut off at the line budget; use browser_find to reach the rest".into());
        }
        Ok(lines)
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(
        &mut self,
        node: &Value,
        by_id: &HashMap<&str, &Value>,
        depth: usize,
        parent_name: &str,
        lines: &mut Vec<String>,
        seen: &mut HashMap<String, Node>,
        max_lines: usize,
    ) {
        if lines.len() >= max_lines {
            return;
        }
        let role = node["role"]["value"].as_str().unwrap_or_default();
        let name = clean(node["name"]["value"].as_str().unwrap_or_default());
        let ignored = node["ignored"].as_bool().unwrap_or(false);
        let states = states(node);
        let interactive = INTERACTIVE.contains(&role);
        let shown = !ignored
            && !LAYOUT.contains(&role)
            && (interactive || !name.is_empty() || !states.is_empty())
            && !(role == "StaticText" && parent_name.contains(&name));
        let (next, child_parent) = match (shown, node["backendDOMNodeId"].as_i64()) {
            (true, Some(backend)) => {
                let id = match self.ids.get(&backend) {
                    Some(id) => id.clone(),
                    None => {
                        self.counter += 1;
                        let id = format!("b{}", self.counter);
                        self.ids.insert(backend, id.clone());
                        id
                    }
                };
                let role = if role == "StaticText" { "text" } else { role };
                lines.push(
                    [format!(
                        "{}[{id}] {role} {}",
                        "  ".repeat(depth + 1),
                        quote(&name)
                    )]
                    .into_iter()
                    .chain(states)
                    .collect::<Vec<_>>()
                    .join(" "),
                );
                seen.insert(
                    id,
                    Node {
                        backend,
                        role: role.to_string(),
                        name: name.clone(),
                    },
                );
                (depth + 1, name)
            }
            _ => (depth, parent_name.to_string()),
        };
        for child in node["childIds"].as_array().into_iter().flatten() {
            if let Some(child) = child.as_str().and_then(|id| by_id.get(id)) {
                self.visit(child, by_id, next, &child_parent, lines, seen, max_lines);
            }
        }
    }

    fn center(&mut self, backend: i64) -> Result<(f64, f64)> {
        let model = self.send("DOM.getBoxModel", json!({"backendNodeId":backend}))?;
        let quad = model["model"]["content"]
            .as_array()
            .context("no box")?
            .iter()
            .filter_map(Value::as_f64)
            .collect::<Vec<_>>();
        ensure!(quad.len() == 8, "no box");
        Ok((
            (quad[0] + quad[2] + quad[4] + quad[6]) / 4.0,
            (quad[1] + quad[3] + quad[5] + quad[7]) / 4.0,
        ))
    }

    fn call_on(&mut self, backend: i64, function: &str, arguments: Value) -> Result<Value> {
        let object = self.send("DOM.resolveNode", json!({"backendNodeId":backend}))?;
        let id = object["object"]["objectId"]
            .as_str()
            .context("element is gone")?
            .to_string();
        let result = self.send(
            "Runtime.callFunctionOn",
            json!({"objectId":id,"functionDeclaration":function,"arguments":arguments,"returnByValue":true}),
        )?;
        Ok(result["result"]["value"].clone())
    }

    fn wait_downloads(&mut self, wait: bool) -> Result<String> {
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            self.pump(Duration::from_millis(if wait { 500 } else { 0 }));
            let pending = self
                .downloads
                .iter()
                .any(|download| download.state == "inProgress");
            if !wait || !pending || Instant::now() >= deadline {
                break;
            }
        }
        if self.downloads.is_empty() {
            return Ok("No downloads in this browser session".into());
        }
        Ok(self
            .downloads
            .iter()
            .map(Download::line)
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// Reads events that arrived since the last command, waiting up to `wait` for the first one.
    fn pump(&mut self, wait: Duration) {
        let timeout = Some(wait.max(Duration::from_millis(1)));
        let _ = self.socket.get_mut().set_read_timeout(timeout);
        while let Ok(Message::Text(text)) = self.socket.read() {
            if let Ok(value) = serde_json::from_str::<Value>(text.as_str()) {
                self.event(&value);
            }
        }
        let _ = self
            .socket
            .get_mut()
            .set_read_timeout(Some(COMMAND_TIMEOUT));
    }

    fn event(&mut self, event: &Value) {
        let params = &event["params"];
        match event["method"].as_str() {
            Some("Browser.downloadWillBegin") => self.downloads.push(Download {
                guid: params["guid"].as_str().unwrap_or_default().into(),
                file: params["suggestedFilename"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
                path: None,
                reported: false,
                state: "inProgress".into(),
                received: 0,
                total: 0,
            }),
            Some("Browser.downloadProgress") => {
                if let Some(download) = self
                    .downloads
                    .iter_mut()
                    .find(|download| download.guid == params["guid"].as_str().unwrap_or_default())
                {
                    download.state = params["state"].as_str().unwrap_or("inProgress").into();
                    download.received = params["receivedBytes"].as_f64().unwrap_or_default() as u64;
                    download.total = params["totalBytes"].as_f64().unwrap_or_default() as u64;
                    if let Some(path) = params["filePath"].as_str() {
                        download.path = Some(path.into());
                    }
                }
            }
            _ => {}
        }
    }

    fn send(&mut self, method: &str, params: Value) -> Result<Value> {
        let session = self.session.clone();
        self.command(Some(&session), method, params)
    }

    fn command(&mut self, session: Option<&str>, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        let id = self.next;
        let mut message = json!({"id":id,"method":method,"params":params});
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        self.socket
            .send(Message::text(message.to_string()))
            .map_err(|error| anyhow!("Chrome connection: {error}"))?;
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            ensure!(Instant::now() < deadline, "{method} timed out");
            let Message::Text(text) = self
                .socket
                .read()
                .map_err(|error| anyhow!("Chrome connection: {error}"))?
            else {
                continue;
            };
            let value: Value = serde_json::from_str(text.as_str())?;
            if value["id"].as_u64() == Some(id) {
                if let Some(error) = value.get("error") {
                    bail!(
                        "{method}: {}",
                        error["message"].as_str().unwrap_or("failed")
                    );
                }
                return Ok(value["result"].clone());
            }
            self.event(&value);
        }
    }
}

impl Download {
    fn line(&self) -> String {
        let path = self.path.clone().unwrap_or_else(|| {
            downloads_dir()
                .map(|dir| dir.join(&self.file).display().to_string())
                .unwrap_or_default()
        });
        match self.state.as_str() {
            "completed" => format!("download finished: {path}"),
            "canceled" => format!("download canceled: {}", self.file),
            _ if self.total > 0 => format!(
                "downloading {} ({}%)",
                self.file,
                self.received * 100 / self.total
            ),
            _ => format!("downloading {}", self.file),
        }
    }
}

fn states(node: &Value) -> Vec<String> {
    let mut states = Vec::new();
    if let Some(value) = node["value"]["value"]
        .as_str()
        .filter(|value| !value.is_empty())
    {
        states.push(format!("value={:?}", clip(value, 80)));
    }
    for property in node["properties"].as_array().into_iter().flatten() {
        let name = property["name"].as_str().unwrap_or_default();
        let value = &property["value"]["value"];
        match (name, value) {
            ("focused", Value::Bool(true)) => states.push("focused".into()),
            ("disabled", Value::Bool(true)) => states.push("disabled".into()),
            ("checked", Value::String(state)) if state == "true" => states.push("checked".into()),
            ("checked", Value::String(state)) if state == "false" => {
                states.push("unchecked".into())
            }
            ("expanded", Value::Bool(expanded)) => {
                states.push(if *expanded { "expanded" } else { "collapsed" }.into())
            }
            ("selected", Value::Bool(true)) => states.push("selected".into()),
            ("required", Value::Bool(true)) => states.push("required".into()),
            _ => {}
        }
    }
    states
}

/// Quotes page text without Rust's debug escapes, which mangle combining marks in many scripts.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn host(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
}

fn clean(text: &str) -> String {
    clip(&text.split_whitespace().collect::<Vec<_>>().join(" "), 120)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max).collect::<String>())
}

fn profile() -> Result<PathBuf> {
    let dir = crate::local_dir()
        .context("LOCALAPPDATA is not set")?
        .join("browser");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn downloads_dir() -> Result<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("USERPROFILE").context("USERPROFILE is not set")?)
        .join("Downloads");
    Ok(dir)
}

/// Chrome, else Edge, which speaks the same protocol.
fn chrome() -> Result<PathBuf> {
    [
        ("ProgramFiles", r"Google\Chrome\Application\chrome.exe"),
        ("ProgramFiles(x86)", r"Google\Chrome\Application\chrome.exe"),
        ("LOCALAPPDATA", r"Google\Chrome\Application\chrome.exe"),
        (
            "ProgramFiles(x86)",
            r"Microsoft\Edge\Application\msedge.exe",
        ),
        ("ProgramFiles", r"Microsoft\Edge\Application\msedge.exe"),
    ]
    .into_iter()
    .filter_map(|(root, path)| Some(PathBuf::from(std::env::var_os(root)?).join(path)))
    .find(|path| path.exists())
    .ok_or_else(|| anyhow!("Chrome or Edge is not installed"))
}

#[cfg(test)]
mod tests {
    use super::host;

    #[test]
    fn host_is_the_site_part_of_a_url() {
        assert_eq!(host("https://cursor.com/download?os=win"), "cursor.com");
        assert_eq!(host("cursor.com/download"), "cursor.com");
        assert_eq!(host("about:blank"), "about:blank");
    }
}
