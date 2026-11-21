//! Desktop control on macOS and Linux: the same tools, text format and safety rules as the Windows
//! version (`uia.rs`), over a [`Backend`] for the platform's accessibility API (`mac.rs` reads
//! macOS Accessibility, `linux.rs` reads AT-SPI). Element IDs, snapshots and diffs, name lookup,
//! batches, approvals, blocked apps, the audit log and the stop switch live here, once.
#![cfg_attr(windows, allow(dead_code))]

use std::{
    collections::{HashMap, HashSet},
    io::Write,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow, bail, ensure};
use serde_json::Value;

use crate::{
    Output, Settings,
    ax::quote,
    common::{Rect, changes, clean, clip, line_id, stopped, summary},
};

/// Most children shown per element before the rest are only counted.
const SIBLINGS: usize = 40;
const POLL: Duration = Duration::from_millis(80);
const QUIET: Duration = Duration::from_millis(400);
const SETTLE_LIMIT: Duration = Duration::from_secs(3);

/// A top-level window as the platform reports it.
pub struct Window<N> {
    /// A stable hexadecimal key, shown as `[w<key>]`.
    pub key: String,
    pub title: String,
    /// The app's name, matched by window queries and the blocked apps list.
    pub app: String,
    pub focused: bool,
    pub bounds: Option<Rect>,
    pub node: N,
}

/// What a snapshot line shows of one element.
#[derive(Default)]
pub struct Info {
    /// Lowercase and named like the Windows roles (`button`, `edit`, `tabitem`), so prompts and
    /// tool calls read the same on every OS.
    pub role: String,
    pub name: String,
    /// The value of a field, slider or similar, when it has one.
    pub value: Option<String>,
    /// Extra words such as `disabled`, `checked`, `selected`, `expanded`.
    pub states: Vec<String>,
    pub password: bool,
    /// Offscreen or otherwise invisible: skipped with its subtree.
    pub hidden: bool,
}

/// One platform's accessibility API and input.
pub trait Backend {
    type Node: Clone;
    /// Visible top-level windows, front to back.
    fn windows(&mut self) -> Result<Vec<Window<Self::Node>>>;
    fn children(&mut self, node: &Self::Node) -> Vec<Self::Node>;
    fn info(&mut self, node: &Self::Node) -> Option<Info>;
    /// Identifies the same element across reads, so it keeps its ID.
    fn identity(&mut self, node: &Self::Node) -> Option<String>;
    /// click, double_click, right_click, toggle, select, add_to_selection, expand, collapse,
    /// scroll_into_view, scroll_down, scroll_up or focus.
    fn perform(&mut self, node: &Self::Node, action: &str) -> Result<()>;
    /// Sets a field's whole text; false when the element takes no text this way.
    fn set_text(&mut self, node: &Self::Node, text: &str) -> Result<bool>;
    /// An element's whole text, for `read`.
    fn text(&mut self, node: &Self::Node) -> String;
    fn raise(&mut self, window: &Window<Self::Node>) -> Result<()>;
    fn press(&mut self, press: &Press) -> Result<()>;
    fn type_text(&mut self, text: &str) -> Result<()>;
    fn click(&mut self, x: i32, y: i32, button: &str) -> Result<()>;
    /// Opens an app, file or URL the way the platform's launcher does.
    fn launch(&mut self, app: &str) -> Result<()>;
    fn screenshot(&mut self, window: &Window<Self::Node>) -> Result<Vec<u8>>;
    /// Lines of text ending with `@x,y`, their centre on screen, for click_at.
    fn ocr(&mut self, window: &Window<Self::Node>) -> Result<Vec<String>>;
    /// The app under a screen point, for approvals.
    fn app_at(&mut self, x: i32, y: i32) -> Option<String>;
}

struct Target<N> {
    node: N,
    window: String,
    name: String,
    role: String,
    label: String,
    password: bool,
}

pub struct Desktop<B: Backend> {
    backend: B,
    settings: Settings,
    ids: HashMap<String, String>,
    next: usize,
    targets: HashMap<String, Target<B::Node>>,
    /// Previous snapshot lines per window key, for diff mode.
    last: HashMap<String, Vec<String>>,
    browser: Option<crate::browser::Browser>,
}

impl<B: Backend> Desktop<B> {
    pub fn with(backend: B, settings: Settings) -> Self {
        Self {
            backend,
            settings,
            ids: HashMap::new(),
            next: 0,
            targets: HashMap::new(),
            last: HashMap::new(),
            browser: None,
        }
    }

    pub fn configure(&mut self, settings: Settings) {
        self.settings = settings;
    }

    pub fn begin_turn(&mut self) {
        crate::common::STOPPED.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn end_turn(&mut self) {}

    pub fn call(&mut self, command: &str, args: &Value) -> Result<Output> {
        let text = |key: &str| args.get(key).and_then(Value::as_str);
        let required = |key: &str| text(key).ok_or_else(|| anyhow!("{command} needs {key}"));
        let number = |key: &str| {
            args.get(key)
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow!("{command} needs {key}"))
        };
        let output = match command {
            "windows" => self.windows(),
            "snapshot" => self.snapshot(
                text("window"),
                args.get("diff").and_then(Value::as_bool).unwrap_or(false),
            ),
            "find" => self.find(required("text")?, text("role"), text("window")),
            "act" => {
                let id = match text("id") {
                    Some(id) => id.to_string(),
                    // A role alone, such as the document in Notepad, means the first element
                    // with that role: models often name the role rather than the element.
                    None => self.resolve(
                        text("name")
                            .or(text("role").map(|_| ""))
                            .ok_or_else(|| anyhow!("act needs id, name or role"))?,
                        text("role"),
                        text("window"),
                    )?,
                };
                self.act(&id, required("action")?, text("value"))
            }
            "batch" => self.batch(
                args.get("steps")
                    .and_then(Value::as_array)
                    .ok_or_else(|| anyhow!("batch needs steps"))?,
            ),
            "key" => self.key(required("keys")?, text("window")),
            "focus" => self.focus(required("window")?),
            "launch" => self.launch(required("app")?),
            "wait_for" => self.wait_for(
                required("text")?,
                text("window"),
                Duration::from_millis(
                    args.get("timeout_ms")
                        .and_then(Value::as_u64)
                        .unwrap_or(10_000)
                        .min(120_000),
                ),
            ),
            "ocr" => self.ocr(text("window")),
            "click_at" => self.click_at(
                number("x")? as i32,
                number("y")? as i32,
                text("button").unwrap_or("left"),
            ),
            "screenshot" => {
                let window = self.window(text("window"))?;
                self.blocked(&window)?;
                let jpeg = self.backend.screenshot(&window)?;
                return Ok(Output::Image {
                    header: header(&window),
                    jpeg,
                });
            }
            command if command.starts_with("browser_") => {
                let (max_lines, passwords) =
                    (self.settings.max_lines, self.settings.type_into_passwords);
                self.browser()?
                    .call(command, args, max_lines, passwords, None)
            }
            _ => Err(anyhow!("Unknown desktop tool {command:?}")),
        };
        output.map(Output::Text)
    }

    fn browser(&mut self) -> Result<&mut crate::browser::Browser> {
        if !self.browser.as_mut().is_some_and(|browser| browser.alive()) {
            self.browser = Some(crate::browser::Browser::connect()?);
        }
        self.browser
            .as_mut()
            .ok_or_else(|| anyhow!("the browser is not connected"))
    }

    /// A human sentence for an approval card, with a warning when the app reaches far beyond
    /// itself (a shell, every file, system settings).
    pub fn describe(&mut self, command: &str, args: &Value) -> String {
        let steps = match command {
            "batch" => args
                .get("steps")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|step| (step["tool"].as_str().unwrap_or_default(), step))
                .collect(),
            _ => vec![(command, args)],
        };
        let warnings = steps
            .into_iter()
            .filter_map(|(tool, step)| {
                let app = match tool {
                    "launch" => step["app"].as_str().map(str::to_lowercase),
                    _ => self.app(tool, step),
                }?;
                reach(&app).map(|reach| format!("{app} {reach}"))
            })
            .collect::<std::collections::BTreeSet<_>>();
        let sentence = self.sentence(command, args);
        if warnings.is_empty() {
            return sentence;
        }
        format!(
            "{sentence} (warning: {})",
            warnings.into_iter().collect::<Vec<_>>().join("; ")
        )
    }

    fn sentence(&mut self, command: &str, args: &Value) -> String {
        if command.starts_with("browser_") {
            return match self.browser() {
                Ok(browser) => browser.describe(command, args),
                Err(_) => command.replace('_', " "),
            };
        }
        let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
        let window = |desktop: &mut Self| {
            desktop
                .window(args.get("window").and_then(Value::as_str))
                .map(|window| format!("{:?} ({})", window.title, window.app))
                .unwrap_or_else(|_| "an unknown window".into())
        };
        match command {
            "act" if text("id").is_empty() => format!(
                "{} {} {:?} in {}{}",
                text("action"),
                args.get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("element"),
                text("name"),
                window(self),
                match text("action") {
                    "type" => format!(": {:?}", clip(text("value"), 200)),
                    _ => String::new(),
                }
            ),
            "batch" => args
                .get("steps")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|step| self.sentence(step["tool"].as_str().unwrap_or_default(), step))
                .collect::<Vec<_>>()
                .join("; then "),
            "act" => match self
                .targets
                .get(text("id"))
                .map(|target| (target.window.clone(), target.label.clone(), target.password))
            {
                Some((window, label, password)) => {
                    let app = self
                        .window(Some(&format!("w{window}")))
                        .map(|window| window.app)
                        .unwrap_or_default();
                    let typed = match (text("action"), password) {
                        ("type", true) => ": (password hidden)".to_string(),
                        ("type", false) => format!(": {:?}", clip(text("value"), 200)),
                        _ => String::new(),
                    };
                    format!("{} {label} in {app}{typed}", text("action"))
                }
                None => format!("{} unknown element {}", text("action"), text("id")),
            },
            "key" => format!("press {:?} in {}", text("keys"), window(self)),
            "focus" => format!("bring {} to the front", window(self)),
            "launch" => format!("open {:?}", text("app")),
            "click_at" => {
                let (x, y) = (
                    args["x"].as_i64().unwrap_or_default() as i32,
                    args["y"].as_i64().unwrap_or_default() as i32,
                );
                let under = self
                    .backend
                    .app_at(x, y)
                    .map(|app| format!(" on {app}"))
                    .unwrap_or_default();
                format!("click at {x},{y}{under}")
            }
            _ => command.to_string(),
        }
    }

    /// The app a command would act on, for "ask once per app". Browser commands count each
    /// website as its own app.
    pub fn app(&mut self, command: &str, args: &Value) -> Option<String> {
        if command.starts_with("browser_") {
            return self
                .browser()
                .ok()?
                .site(command, args)
                .map(|site| format!("site {site}"));
        }
        let text = |key: &str| args.get(key).and_then(Value::as_str);
        let app = match command {
            "act" if text("id").is_none() => self.window(text("window")).ok()?.app,
            "batch" => {
                let apps = args
                    .get("steps")?
                    .as_array()?
                    .iter()
                    .filter(|step| crate::acts(step["tool"].as_str().unwrap_or_default()))
                    .map(|step| self.app(step["tool"].as_str().unwrap_or_default(), step))
                    .collect::<Option<HashSet<_>>>()?;
                return (apps.len() == 1).then(|| apps.into_iter().next()).flatten();
            }
            "act" => {
                let window = self.targets.get(text("id")?)?.window.clone();
                self.window(Some(&format!("w{window}"))).ok()?.app
            }
            "key" | "focus" => self.window(text("window")).ok()?.app,
            "click_at" => self
                .backend
                .app_at(args["x"].as_i64()? as i32, args["y"].as_i64()? as i32)?,
            _ => return None,
        };
        Some(app.to_lowercase()).filter(|app| !app.is_empty())
    }

    pub fn windows(&mut self) -> Result<String> {
        let lines = self
            .backend
            .windows()?
            .iter()
            .filter(|window| !window.title.is_empty())
            .map(|window| {
                let size = window
                    .bounds
                    .map(|rect| format!(" {}x{}", rect.right - rect.left, rect.bottom - rect.top))
                    .unwrap_or_default();
                let mark = if window.focused { " *focused" } else { "" };
                format!(
                    "[w{}] {:?} ({}){size}{mark}",
                    window.key,
                    clean(&window.title),
                    window.app
                )
            })
            .collect::<Vec<_>>();
        Ok(lines.join("\n"))
    }

    pub fn snapshot(&mut self, window: Option<&str>, diff: bool) -> Result<String> {
        let window = self.window(window)?;
        self.blocked(&window)?;
        let (lines, truncated) = self.read(&window);
        let header = header(&window);
        let previous = self.last.insert(window.key.clone(), lines.clone());
        let ocr = self.ocr_note(&window, &lines);
        if let (true, Some(previous)) = (diff, previous) {
            return Ok(format!("{}{ocr}", changes(&header, &previous, &lines)));
        }
        let note = if truncated {
            "\n  … cut off at the line budget; use find to reach the rest"
        } else {
            ""
        };
        Ok(format!("{header}\n{}{note}{ocr}", lines.join("\n")))
    }

    pub fn find(&mut self, text: &str, role: Option<&str>, window: Option<&str>) -> Result<String> {
        let window = self.window(window)?;
        self.blocked(&window)?;
        let lines = self.scan(
            &window,
            Some((text.to_lowercase(), role.map(str::to_lowercase))),
            self.settings.max_lines,
        );
        if lines.0.is_empty() {
            return Ok(format!(
                "No element matching {text:?} in {}",
                header(&window)
            ));
        }
        Ok(lines
            .0
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// An exact (case-insensitive) name match first, else the first element containing it.
    fn resolve(&mut self, name: &str, role: Option<&str>, window: Option<&str>) -> Result<String> {
        let window = self.window(window)?;
        self.blocked(&window)?;
        let wanted = name.to_lowercase();
        let role = role.map(str::to_lowercase);
        let (lines, _) = self.scan(&window, Some((wanted.clone(), role.clone())), 400);
        let ids = lines
            .iter()
            .filter_map(|(_, line)| line_id(line))
            .collect::<Vec<_>>();
        let exact = |target: &Target<B::Node>| {
            target.name.to_lowercase() == wanted
                && role.as_ref().is_none_or(|role| *role == target.role)
        };
        ids.iter()
            .find(|id| self.targets.get(id.as_str()).is_some_and(exact))
            .or(ids.first())
            .cloned()
            .ok_or_else(|| anyhow!("No element matching {name:?} in {}", header(&window)))
    }

    pub fn batch(&mut self, steps: &[Value]) -> Result<String> {
        let mut results = Vec::new();
        for (index, step) in steps.iter().enumerate() {
            let tool = step["tool"].as_str().unwrap_or_default();
            let result = match tool {
                "batch" | "screenshot" => Err(anyhow!("{tool} cannot run inside batch")),
                _ => self.call(tool, step),
            };
            match result {
                Ok(Output::Text(text)) if crate::acts(tool) && index + 1 < steps.len() => {
                    results.push(format!("{}. {tool}: {}", index + 1, summary(&text)))
                }
                Ok(Output::Text(text)) => results.push(format!("{}. {tool}: {text}", index + 1)),
                Ok(Output::Image { .. }) => unreachable!("screenshot is refused above"),
                Err(error) => {
                    results.push(format!(
                        "{}. {tool} failed: {error:#}\nThe remaining {} step(s) did not run.",
                        index + 1,
                        steps.len() - index - 1
                    ));
                    bail!("{}", results.join("\n"));
                }
            }
        }
        Ok(results.join("\n"))
    }

    pub fn act(&mut self, id: &str, action: &str, value: Option<&str>) -> Result<String> {
        stopped()?;
        let target = self
            .targets
            .get(id)
            .ok_or_else(|| anyhow!("Unknown element {id}. Take a new snapshot."))?;
        let node = target.node.clone();
        let label = format!("[{id}] {}", target.label);
        let password = target.password;
        let window = self.window(Some(&format!("w{}", target.window)))?;
        self.blocked(&window)?;
        if action == "read" {
            ensure!(
                !password,
                "{label} is a password field; its text is never read."
            );
            return Ok(format!("{label} text:\n{}", self.backend.text(&node)));
        }
        match action {
            "type" => {
                let text = value.ok_or_else(|| anyhow!("type needs a value"))?;
                if password && !self.settings.type_into_passwords {
                    bail!(
                        "Typing into password fields is off in desktop-control settings (typeIntoPasswords)."
                    );
                }
                if !self.backend.set_text(&node, text)? {
                    self.backend.perform(&node, "focus")?;
                    self.backend.type_text(text)?;
                }
            }
            "click" | "double_click" | "right_click" | "toggle" | "select" | "add_to_selection"
            | "expand" | "collapse" | "scroll_into_view" | "scroll_down" | "scroll_up"
            | "focus" => self.backend.perform(&node, action)?,
            _ => bail!("Unknown action {action:?}"),
        }
        self.audit(&window.app, &label, action);
        Ok(format!(
            "{action} {label}: done\n{}",
            self.after(&window.key)
        ))
    }

    pub fn key(&mut self, keys: &str, window: Option<&str>) -> Result<String> {
        stopped()?;
        let window = self.window(window)?;
        self.blocked(&window)?;
        self.backend.raise(&window)?;
        for press in presses(keys)? {
            stopped()?;
            match press {
                Step::Text(text) => self.backend.type_text(&text)?,
                Step::Press(press) => self.backend.press(&press)?,
            }
        }
        self.audit(&window.app, "keys", "key");
        Ok(format!("sent keys\n{}", self.after(&window.key)))
    }

    pub fn focus(&mut self, window: &str) -> Result<String> {
        stopped()?;
        let window = self.window(Some(window))?;
        self.blocked(&window)?;
        self.backend.raise(&window)?;
        Ok(format!("focused {}", header(&window)))
    }

    pub fn launch(&mut self, app: &str) -> Result<String> {
        stopped()?;
        if let Some(entry) = self.blocked_entry(app, app) {
            bail!("{entry} is on the blocked apps list in desktop-control settings.");
        }
        let before = self.foreground_key();
        self.backend.launch(app)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(250));
            if let Some(window) = self
                .foreground()
                .filter(|window| Some(&window.key) != before.as_ref())
            {
                std::thread::sleep(QUIET);
                return Ok(format!("opened {}", header(&window)));
            }
        }
        Ok(format!(
            "asked the system to open {app:?}; no new window came to the front within 10 seconds"
        ))
    }

    pub fn ocr(&mut self, window: Option<&str>) -> Result<String> {
        let window = self.window(window)?;
        self.blocked(&window)?;
        let lines = self.backend.ocr(&window)?;
        Ok(format!("{} (OCR)\n{}", header(&window), lines.join("\n")))
    }

    pub fn click_at(&mut self, x: i32, y: i32, button: &str) -> Result<String> {
        stopped()?;
        let app = self.backend.app_at(x, y).unwrap_or_default();
        if let Some(entry) = self.blocked_entry(&app, &app) {
            bail!("{entry} is on the blocked apps list in desktop-control settings.");
        }
        self.backend.click(x, y, button)?;
        self.audit(&app, &format!("point {x},{y}"), "click_at");
        let after = match self.foreground() {
            Some(window) => self.after(&window.key),
            None => String::new(),
        };
        Ok(format!("clicked {x},{y}\n{after}"))
    }

    pub fn wait_for(
        &mut self,
        text: &str,
        window: Option<&str>,
        timeout: Duration,
    ) -> Result<String> {
        let deadline = Instant::now() + timeout;
        loop {
            let target = self.window(window)?;
            self.blocked(&target)?;
            let (lines, _) = self.scan(&target, Some((text.to_lowercase(), None)), 50);
            if !lines.is_empty() {
                return Ok(lines
                    .into_iter()
                    .map(|(_, line)| line)
                    .collect::<Vec<_>>()
                    .join("\n"));
            }
            ensure!(
                Instant::now() < deadline,
                "{text:?} did not appear within {} ms",
                timeout.as_millis()
            );
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    /// A window's lines, indented, and whether they were cut off.
    fn read(&mut self, window: &Window<B::Node>) -> (Vec<String>, bool) {
        let (lines, truncated) = self.scan(window, None, self.settings.max_lines);
        (
            lines
                .into_iter()
                .map(|(depth, line)| format!("{}{line}", "  ".repeat(depth + 1)))
                .collect(),
            truncated,
        )
    }

    /// Walks a window's tree into lines. With a filter (lowercase text, optional role) it lists
    /// matches flat instead.
    fn scan(
        &mut self,
        window: &Window<B::Node>,
        filter: Option<(String, Option<String>)>,
        limit: usize,
    ) -> (Vec<(usize, String)>, bool) {
        enum Visit<N> {
            Node {
                node: N,
                depth: usize,
                parent: String,
                root: bool,
            },
            /// "… N more" after the first SIBLINGS children of an element.
            More(usize, usize),
        }
        let mut lines = Vec::new();
        let mut truncated = false;
        let mut stack = vec![Visit::Node {
            node: window.node.clone(),
            depth: 0,
            parent: String::new(),
            root: true,
        }];
        // Depth first with children in order, so they go on the stack reversed.
        while let Some(visit) = stack.pop() {
            if lines.len() >= limit {
                truncated = true;
                break;
            }
            let (node, depth, parent, root) = match visit {
                Visit::More(depth, count) => {
                    lines.push((depth, format!("… {count} more")));
                    continue;
                }
                Visit::Node {
                    node,
                    depth,
                    parent,
                    root,
                } => (node, depth, parent, root),
            };
            let (next, name) = if root {
                (0, window.title.clone())
            } else {
                match self.line(&node, &window.key, &parent, filter.as_ref()) {
                    Some(Line::Shown(line, name)) => {
                        lines.push((depth, line));
                        (if filter.is_some() { 0 } else { depth + 1 }, name)
                    }
                    Some(Line::Skipped) => (depth, parent),
                    None => continue,
                }
            };
            let mut children = self.backend.children(&node);
            if filter.is_none() && children.len() > SIBLINGS {
                stack.push(Visit::More(next, children.len() - SIBLINGS));
                children.truncate(SIBLINGS);
            }
            for child in children.into_iter().rev() {
                stack.push(Visit::Node {
                    node: child,
                    depth: next,
                    parent: name.clone(),
                    root: false,
                });
            }
        }
        (lines, truncated)
    }

    /// A line for an element, Skipped for an unnamed container whose children still print, or
    /// None for a hidden element and its subtree.
    fn line(
        &mut self,
        node: &B::Node,
        window: &str,
        parent_name: &str,
        filter: Option<&(String, Option<String>)>,
    ) -> Option<Line> {
        let info = self.backend.info(node)?;
        if info.hidden {
            return None;
        }
        let name = clean(&info.name);
        let mut states = info.states.clone();
        if let Some(value) = info.value.as_ref().filter(|value| !value.is_empty()) {
            if info.password {
                states.insert(0, "value=(hidden)".into());
            } else {
                states.insert(0, format!("value={}", quote(&clip(&clean(value), 80))));
            }
        }
        let interactive = matches!(
            info.role.as_str(),
            "button"
                | "edit"
                | "checkbox"
                | "combobox"
                | "listitem"
                | "menuitem"
                | "tabitem"
                | "radiobutton"
                | "hyperlink"
                | "slider"
                | "treeitem"
                | "document"
        );
        if (name.is_empty() && states.is_empty() && !interactive)
            || (info.role == "text" && !name.is_empty() && parent_name.starts_with(&name))
        {
            return Some(Line::Skipped);
        }
        if let Some((text, role)) = filter {
            let haystack = format!(
                "{} {}",
                name.to_lowercase(),
                states.join(" ").to_lowercase()
            );
            if !haystack.contains(text.as_str())
                || role.as_ref().is_some_and(|role| *role != info.role)
            {
                return Some(Line::Skipped);
            }
        }
        let id = self.id_for(node);
        let label = format!("{} {}", info.role, quote(&name));
        let line = [format!("[{id}] {label}")]
            .into_iter()
            .chain(states)
            .collect::<Vec<_>>()
            .join(" ");
        self.targets.insert(
            id,
            Target {
                node: node.clone(),
                window: window.to_string(),
                name: name.clone(),
                role: info.role,
                label,
                password: info.password,
            },
        );
        Some(Line::Shown(line, name))
    }

    fn id_for(&mut self, node: &B::Node) -> String {
        let key = self.backend.identity(node);
        if let Some(id) = key.as_ref().and_then(|key| self.ids.get(key)) {
            return id.clone();
        }
        self.next += 1;
        let id = format!("e{}", self.next);
        if let Some(key) = key {
            self.ids.insert(key, id.clone());
        }
        id
    }

    /// What changed in the acted-on window once it settles, plus the new foreground window.
    fn after(&mut self, key: &str) -> String {
        let changes = match self.window(Some(&format!("w{key}"))) {
            Err(_) => {
                self.last.remove(key);
                format!("window [w{key}] closed")
            }
            Ok(window) => self.settle(&window),
        };
        match self.foreground() {
            Some(window) if window.key != key => {
                format!(
                    "{changes}\nforeground is now [w{}] {}",
                    window.key,
                    header(&window)
                )
            }
            _ => changes,
        }
    }

    fn settle(&mut self, window: &Window<B::Node>) -> String {
        let deadline = Instant::now() + SETTLE_LIMIT;
        std::thread::sleep(POLL);
        let mut lines = self.read(window).0;
        let mut quiet = Instant::now();
        while quiet.elapsed() < QUIET && Instant::now() < deadline {
            std::thread::sleep(POLL);
            let next = self.read(window).0;
            if next != lines {
                lines = next;
                quiet = Instant::now();
            }
        }
        let header = header(window);
        let ocr = self.ocr_note(window, &lines);
        match self.last.insert(window.key.clone(), lines.clone()) {
            Some(previous) => format!("{}{ocr}", changes(&header, &previous, &lines)),
            None => format!("{header}\n{}{ocr}", lines.join("\n")),
        }
    }

    /// Windows that show almost nothing (games, canvases, video) are read by OCR as well.
    fn ocr_note(&mut self, window: &Window<B::Node>, lines: &[String]) -> String {
        let named = lines
            .iter()
            .filter_map(|line| line.split('"').nth(1))
            .filter(|name| !name.is_empty())
            .count();
        if named >= 3 {
            return String::new();
        }
        match self.backend.ocr(window) {
            Ok(found) if found.is_empty() => {
                "\n  (little accessibility text, and OCR found none)".into()
            }
            Ok(found) => format!(
                "\n  read by OCR because the window shows little accessibility text (click_at x,y):\n{}",
                found
                    .iter()
                    .map(|line| format!("  {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            Err(error) => format!("\n  (little accessibility text; OCR: {error:#})"),
        }
    }

    fn foreground(&mut self) -> Option<Window<B::Node>> {
        self.backend
            .windows()
            .ok()?
            .into_iter()
            .find(|window| window.focused)
    }

    fn foreground_key(&mut self) -> Option<String> {
        self.foreground().map(|window| window.key)
    }

    /// `w<key>`, a title or an app name (exact first, then contained); None is the foreground.
    fn window(&mut self, query: Option<&str>) -> Result<Window<B::Node>> {
        let windows = self.backend.windows()?;
        let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) else {
            return windows
                .into_iter()
                .find(|window| window.focused)
                .ok_or_else(|| anyhow!("No window is in front. Use windows to list them."));
        };
        if let Some(key) = query.strip_prefix('w')
            && let Some(index) = windows.iter().position(|window| window.key == key)
        {
            return Ok(windows.into_iter().nth(index).unwrap());
        }
        let wanted = query.to_lowercase();
        let exact = windows.iter().position(|window| {
            window.title.to_lowercase() == wanted || window.app.to_lowercase() == wanted
        });
        let contains = || {
            windows.iter().position(|window| {
                window.title.to_lowercase().contains(&wanted)
                    || window.app.to_lowercase().contains(&wanted)
            })
        };
        let index = exact
            .or_else(contains)
            .ok_or_else(|| anyhow!("No window matches {query:?}. Use windows to list them."))?;
        Ok(windows.into_iter().nth(index).unwrap())
    }

    fn blocked(&self, window: &Window<B::Node>) -> Result<()> {
        match self.blocked_entry(&window.title, &window.app) {
            Some(entry) => {
                bail!("{entry} is on the blocked apps list in desktop-control settings.")
            }
            None => Ok(()),
        }
    }

    fn blocked_entry(&self, title: &str, app: &str) -> Option<String> {
        let title = title.to_lowercase();
        let app = app.to_lowercase();
        self.settings
            .blocked_apps
            .iter()
            .find(|entry| {
                let entry = entry.to_lowercase();
                !entry.is_empty() && (title.contains(&entry) || app == entry)
            })
            .cloned()
    }

    fn audit(&self, app: &str, label: &str, action: &str) {
        let Some(path) = &self.settings.audit_log else {
            return;
        };
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|time| time.as_secs())
            .unwrap_or_default();
        let line = format!("{seconds}\t{app}\t{label}\t{action}\n");
        // Best effort: a full disk must not block the user's own control session.
        let _ = path.parent().map(std::fs::create_dir_all);
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| file.write_all(line.as_bytes()));
    }
}

enum Line {
    Shown(String, String),
    Skipped,
}

fn header<N>(window: &Window<N>) -> String {
    let size = window
        .bounds
        .map(|rect| format!(" {}x{}", rect.right - rect.left, rect.bottom - rect.top))
        .unwrap_or_default();
    format!(
        "window [w{}] {:?} ({}){size}",
        window.key,
        clean(&window.title),
        window.app
    )
}

/// Apps whose control reaches far beyond themselves, warned about on approval cards.
fn reach(app: &str) -> Option<&'static str> {
    let app = app.trim_end_matches(".app");
    if [
        "terminal",
        "iterm2",
        "iterm",
        "warp",
        "alacritty",
        "kitty",
        "wezterm",
        "ghostty",
        "gnome-terminal",
        "konsole",
        "xterm",
        "tilix",
        "code",
        "cursor",
        "windsurf",
        "zed",
    ]
    .contains(&app)
    {
        return Some("is equivalent to shell access");
    }
    if ["finder", "nautilus", "files", "dolphin", "thunar"].contains(&app) {
        return Some("can read or write any file");
    }
    if [
        "system settings",
        "system preferences",
        "gnome-control-center",
        "settings",
        "systemsettings",
    ]
    .contains(&app)
    {
        return Some("can change system settings");
    }
    None
}

// Keys: the same syntax as on Windows. `{ctrl}s`, `{alt}{f4}`, `{enter}`, `{tab 3}`; `{ctrl}(ab)`
// holds Ctrl for a and b; `{{}` types a brace; other text types literally. On macOS `{cmd}` (also
// `{win}`) is Command.

/// One key press with the modifiers held for it.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Press {
    pub modifiers: Vec<Modifier>,
    pub key: Key,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    /// Command on macOS, Super on Linux.
    Meta,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Key {
    Named(&'static str),
    Char(char),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Text(String),
    Press(Press),
}

const NAMED: &[&str] = &[
    "enter",
    "tab",
    "escape",
    "backspace",
    "delete",
    "space",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
    "insert",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];

/// Parses the key syntax into typed text and key presses.
pub fn presses(keys: &str) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    let mut text = String::new();
    let mut held = Vec::new();
    let mut chars = keys.chars().peekable();
    let flush = |text: &mut String, steps: &mut Vec<Step>| {
        if !text.is_empty() {
            steps.push(Step::Text(std::mem::take(text)));
        }
    };
    while let Some(char) = chars.next() {
        match char {
            '{' => {
                let mut token = String::new();
                for next in chars.by_ref() {
                    if next == '}' && !token.is_empty() {
                        break;
                    }
                    token.push(next);
                }
                if token == "{" || token == "}" {
                    if held.is_empty() {
                        text.push_str(&token);
                    } else {
                        flush(&mut text, &mut steps);
                        steps.push(Step::Press(Press {
                            modifiers: std::mem::take(&mut held),
                            key: Key::Char(token.chars().next().unwrap()),
                        }));
                    }
                    continue;
                }
                let lower = token.to_lowercase();
                let (name, count) = match lower.split_once(' ') {
                    Some((name, count)) => {
                        (name.to_string(), count.trim().parse::<usize>().unwrap_or(1))
                    }
                    None => (lower, 1),
                };
                let modifier = match name.as_str() {
                    "ctrl" | "control" => Some(Modifier::Ctrl),
                    "alt" | "option" => Some(Modifier::Alt),
                    "shift" => Some(Modifier::Shift),
                    "win" | "cmd" | "command" | "meta" | "super" => Some(Modifier::Meta),
                    _ => None,
                };
                flush(&mut text, &mut steps);
                if let Some(modifier) = modifier {
                    held.push(modifier);
                    // `{ctrl}(ab)` holds the modifiers for every key in the group.
                    if chars.peek() == Some(&'(') {
                        chars.next();
                        let modifiers = std::mem::take(&mut held);
                        for grouped in chars.by_ref() {
                            if grouped == ')' {
                                break;
                            }
                            steps.push(Step::Press(Press {
                                modifiers: modifiers.clone(),
                                key: Key::Char(grouped),
                            }));
                        }
                    }
                    continue;
                }
                let key = match NAMED.iter().find(|named| **named == name) {
                    Some(named) => Key::Named(named),
                    None if name.chars().count() == 1 => Key::Char(name.chars().next().unwrap()),
                    None => match name.as_str() {
                        "esc" => Key::Named("escape"),
                        "return" => Key::Named("enter"),
                        "del" => Key::Named("delete"),
                        "bs" | "backspace" => Key::Named("backspace"),
                        "pgup" => Key::Named("pageup"),
                        "pgdn" => Key::Named("pagedown"),
                        _ => bail!("Unknown key {{{token}}}"),
                    },
                };
                let modifiers = std::mem::take(&mut held);
                for _ in 0..count.clamp(1, 100) {
                    steps.push(Step::Press(Press {
                        modifiers: modifiers.clone(),
                        key: key.clone(),
                    }));
                }
            }
            '\n' => {
                flush(&mut text, &mut steps);
                steps.push(Step::Press(Press {
                    modifiers: std::mem::take(&mut held),
                    key: Key::Named("enter"),
                }));
            }
            '\t' => {
                flush(&mut text, &mut steps);
                steps.push(Step::Press(Press {
                    modifiers: std::mem::take(&mut held),
                    key: Key::Named("tab"),
                }));
            }
            char if !held.is_empty() => {
                flush(&mut text, &mut steps);
                steps.push(Step::Press(Press {
                    modifiers: std::mem::take(&mut held),
                    key: Key::Char(char),
                }));
            }
            char => text.push(char),
        }
    }
    flush(&mut text, &mut steps);
    ensure!(
        held.is_empty(),
        "A modifier at the end of the keys has no key to press."
    );
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(modifiers: &[Modifier], key: Key) -> Step {
        Step::Press(Press {
            modifiers: modifiers.to_vec(),
            key,
        })
    }

    #[test]
    fn keys_parse_like_on_windows() {
        assert_eq!(
            presses("hello{enter}").unwrap(),
            vec![Step::Text("hello".into()), press(&[], Key::Named("enter"))]
        );
        assert_eq!(
            presses("{ctrl}s").unwrap(),
            vec![press(&[Modifier::Ctrl], Key::Char('s'))]
        );
        assert_eq!(
            presses("{cmd}{shift}z").unwrap(),
            vec![press(&[Modifier::Meta, Modifier::Shift], Key::Char('z'))]
        );
        assert_eq!(
            presses("{tab 3}").unwrap(),
            vec![
                press(&[], Key::Named("tab")),
                press(&[], Key::Named("tab")),
                press(&[], Key::Named("tab"))
            ]
        );
        assert_eq!(
            presses("{ctrl}(ab)c").unwrap(),
            vec![
                press(&[Modifier::Ctrl], Key::Char('a')),
                press(&[Modifier::Ctrl], Key::Char('b')),
                Step::Text("c".into())
            ]
        );
        assert_eq!(presses("a{{}b").unwrap(), vec![Step::Text("a{b".into())]);
        assert_eq!(
            presses("{alt}{f4}").unwrap(),
            vec![press(&[Modifier::Alt], Key::Named("f4"))]
        );
        assert!(presses("{nope}").is_err());
        assert!(presses("{ctrl}").is_err());
    }

    #[test]
    fn far_reaching_apps_are_warned_about() {
        assert_eq!(reach("terminal"), Some("is equivalent to shell access"));
        assert_eq!(reach("finder"), Some("can read or write any file"));
        assert_eq!(reach("textedit"), None);
    }
}
