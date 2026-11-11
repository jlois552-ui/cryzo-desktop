use std::{
    collections::{HashMap, HashSet},
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow, bail};
use uiautomation::{
    UIAutomation, UIElement, UITreeWalker,
    core::UICacheRequest,
    inputs::{Keyboard, Mouse},
    patterns::{
        UIExpandCollapsePattern, UIInvokePattern, UIScrollItemPattern, UISelectionItemPattern,
        UITogglePattern, UIValuePattern,
    },
    types::{ControlType, Handle, Point, TreeScope, UIProperty},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND},
        System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
        UI::{
            Input::KeyboardAndMouse::{
                MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, VK_ESCAPE,
            },
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                GetForegroundWindow, GetMessageW, MSG, SW_SHOWNORMAL, SetForegroundWindow,
                WM_HOTKEY,
            },
        },
    },
    core::{HSTRING, PCWSTR, PWSTR},
};

use serde_json::Value;

use crate::{Output, Settings, capture::capture};

/// Set by the global stop hotkey. Once set, every acting call fails until the process restarts.
static STOPPED: AtomicBool = AtomicBool::new(false);

/// Everything `line_for` and `states` read, fetched in one cache request per snapshot.
const PROPERTIES: [UIProperty; 15] = [
    UIProperty::ControlType,
    UIProperty::Name,
    UIProperty::IsOffscreen,
    UIProperty::IsEnabled,
    UIProperty::HasKeyboardFocus,
    UIProperty::IsPassword,
    UIProperty::RuntimeId,
    UIProperty::IsValuePatternAvailable,
    UIProperty::ValueValue,
    UIProperty::IsTogglePatternAvailable,
    UIProperty::ToggleToggleState,
    UIProperty::IsSelectionItemPatternAvailable,
    UIProperty::SelectionItemIsSelected,
    UIProperty::IsExpandCollapsePatternAvailable,
    UIProperty::ExpandCollapseExpandCollapseState,
];

/// Siblings printed under one parent before the rest collapse into "… N more".
const SIBLINGS: usize = 40;
/// Settle time after an action before reading what changed.
const SETTLE: Duration = Duration::from_millis(350);

pub struct Desktop {
    automation: UIAutomation,
    walker: UITreeWalker,
    /// Fetches a window's whole control-view subtree, with every property a line needs, in one
    /// cross-process request instead of several calls per element.
    cache: UICacheRequest,
    settings: Settings,
    /// UIA runtime ID → short ID, so an element keeps its ID across snapshots.
    ids: HashMap<Vec<i32>, String>,
    next: usize,
    targets: HashMap<String, Target>,
    /// Previous snapshot lines per window handle, for diff mode.
    last: HashMap<isize, Vec<String>>,
}

struct Target {
    element: UIElement,
    kind: ControlType,
    window: isize,
    label: String,
    password: bool,
}

struct Scan {
    lines: Vec<(usize, String)>,
    limit: usize,
    truncated: bool,
    /// `find` mode: lowercase text and optional role; prints matches flat instead of a tree.
    filter: Option<(String, Option<String>)>,
}

impl Desktop {
    pub fn new(settings: Settings) -> Result<Self> {
        // Physical pixels everywhere, so UIA rectangles, OCR boxes and clicks agree on scaled
        // displays. Fails harmlessly when the host process already chose an awareness.
        let _ =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let automation = UIAutomation::new()?;
        let walker = automation.get_control_view_walker()?;
        let cache = automation.create_cache_request()?;
        for property in PROPERTIES {
            cache.add_property(property)?;
        }
        cache.set_tree_scope(TreeScope::Subtree)?;
        cache.set_tree_filter(automation.get_control_view_condition()?)?;
        Ok(Self {
            automation,
            walker,
            cache,
            settings,
            ids: HashMap::new(),
            next: 0,
            targets: HashMap::new(),
            last: HashMap::new(),
        })
    }

    /// Runs one tool by name with JSON arguments, the shape both MCP and the Cryzo engine use.
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
            "act" => self.act(required("id")?, required("action")?, text("value")),
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
                let (header, jpeg) = self.screenshot(text("window"))?;
                return Ok(Output::Image { header, jpeg });
            }
            _ => Err(anyhow!("Unknown desktop tool {command:?}")),
        };
        output.map(Output::Text)
    }

    /// A human sentence for an approval card, such as `click button "Post now" in chrome.exe`.
    pub fn describe(&self, command: &str, args: &Value) -> String {
        let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
        let window = |key: &str| {
            self.window(args.get(key).and_then(Value::as_str))
                .map(|window| {
                    format!(
                        "{:?} ({})",
                        clean(&window.get_name().unwrap_or_default()),
                        process(&window)
                    )
                })
                .unwrap_or_else(|_| "an unknown window".into())
        };
        match command {
            "act" => match self.targets.get(text("id")) {
                Some(target) => {
                    let app = self
                        .automation
                        .element_from_handle(Handle::from(target.window))
                        .map(|window| process(&window))
                        .unwrap_or_default();
                    let typed = match (text("action"), target.password) {
                        ("type", true) => ": (password hidden)".to_string(),
                        ("type", false) => format!(": {:?}", clip(text("value"), 200)),
                        _ => String::new(),
                    };
                    format!("{} {} in {app}{typed}", text("action"), target.label)
                }
                None => format!("{} unknown element {}", text("action"), text("id")),
            },
            "key" => format!("press {:?} in {}", text("keys"), window("window")),
            "focus" => format!("bring {} to the front", window("window")),
            "launch" => format!("open {:?}", text("app")),
            "click_at" => {
                let (x, y) = (
                    args["x"].as_i64().unwrap_or_default() as i32,
                    args["y"].as_i64().unwrap_or_default() as i32,
                );
                let under = self
                    .automation
                    .element_from_point(Point::new(x, y))
                    .map(|element| {
                        format!(
                            " on {:?} ({})",
                            clean(&element.get_name().unwrap_or_default()),
                            process(&element)
                        )
                    })
                    .unwrap_or_default();
                format!("click at {x},{y}{under}")
            }
            _ => command.to_string(),
        }
    }

    /// The process a command would act on, for "ask once per app".
    pub fn app(&self, command: &str, args: &Value) -> Option<String> {
        let text = |key: &str| args.get(key).and_then(Value::as_str);
        let app = match command {
            "act" => process(
                &self
                    .automation
                    .element_from_handle(Handle::from(self.targets.get(text("id")?)?.window))
                    .ok()?,
            ),
            "key" | "focus" => process(&self.window(text("window")).ok()?),
            "click_at" => process(
                &self
                    .automation
                    .element_from_point(Point::new(
                        args["x"].as_i64()? as i32,
                        args["y"].as_i64()? as i32,
                    ))
                    .ok()?,
            ),
            _ => return None,
        };
        Some(app.to_lowercase()).filter(|app| !app.is_empty())
    }

    /// Top-level windows, foreground first marked with `*`.
    pub fn windows(&self) -> Result<String> {
        let foreground = unsafe { GetForegroundWindow() }.0 as isize;
        let lines = self
            .top_level()?
            .iter()
            .filter(|window| !window.is_offscreen().unwrap_or(true))
            .filter_map(|window| {
                let title = clean(&window.get_name().ok()?);
                if title.is_empty() {
                    return None;
                }
                let hwnd = hwnd(window);
                let rect = window.get_bounding_rectangle().ok()?;
                let mark = if hwnd == foreground { " *focused" } else { "" };
                Some(format!(
                    "[w{hwnd:x}] {title:?} ({}) {}x{}{mark}",
                    process(window),
                    rect.get_width(),
                    rect.get_height()
                ))
            })
            .collect::<Vec<_>>();
        Ok(lines.join("\n"))
    }

    /// Compact text tree of a window (the foreground window when `window` is None). With `diff`,
    /// returns only the lines that changed since this window's previous snapshot.
    pub fn snapshot(&mut self, window: Option<&str>, diff: bool) -> Result<String> {
        let window = self.window(window)?;
        self.blocked(&window)?;
        let hwnd = hwnd(&window);
        let first = self.scan(&window, hwnd, None, self.settings.max_lines);
        // Chromium and WebView2 build their web accessibility tree only after the first UIA
        // client asks, so a near-empty first read is usually just a cold tree.
        let scan = if first.lines.len() < 8 {
            std::thread::sleep(Duration::from_millis(700));
            self.scan(&window, hwnd, None, self.settings.max_lines)
        } else {
            first
        };
        let lines = scan
            .lines
            .iter()
            .map(|(depth, line)| format!("{}{line}", "  ".repeat(depth + 1)))
            .collect::<Vec<_>>();
        let header = header(&window);
        let previous = self.last.insert(hwnd, lines.clone());
        if let (true, Some(previous)) = (diff, previous) {
            return Ok(changes(&header, &previous, &lines));
        }
        let note = if scan.truncated {
            "\n  … cut off at the line or time budget; use find to reach the rest"
        } else {
            ""
        };
        Ok(format!("{header}\n{}{note}", lines.join("\n")))
    }

    /// Elements whose name or value contains `text` (and whose role matches, when given).
    pub fn find(&mut self, text: &str, role: Option<&str>, window: Option<&str>) -> Result<String> {
        let window = self.window(window)?;
        self.blocked(&window)?;
        let filter = (text.to_lowercase(), role.map(str::to_lowercase));
        let scan = self.scan(&window, hwnd(&window), Some(filter), 50);
        if scan.lines.is_empty() {
            return Ok(format!(
                "No element matching {text:?} in {}",
                header(&window)
            ));
        }
        Ok(scan
            .lines
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// Acts on an element from a snapshot, then returns what changed in its window.
    pub fn act(&mut self, id: &str, action: &str, value: Option<&str>) -> Result<String> {
        stopped()?;
        let target = self
            .targets
            .get(id)
            .ok_or_else(|| anyhow!("Unknown element {id}. Take a new snapshot."))?;
        let element = target.element.clone();
        let label = format!("[{id}] {}", target.label);
        let password = target.password;
        let kind = target.kind;
        let window_id = target.window;
        let window = self
            .automation
            .element_from_handle(Handle::from(window_id))?;
        self.blocked(&window)?;
        match action {
            "click" => match element.get_pattern::<UIInvokePattern>() {
                Ok(pattern) => pattern.invoke()?,
                Err(_) => element.click()?,
            },
            "double_click" => element.double_click()?,
            "right_click" => element.right_click()?,
            "type" => {
                let text = value.ok_or_else(|| anyhow!("type needs a value"))?;
                if password && !self.settings.type_into_passwords {
                    bail!(
                        "Typing into password fields is off in desktop-control settings (typeIntoPasswords)."
                    );
                }
                // Documents get real keystrokes so the app's undo history and input events see
                // them; single fields take the value directly, which is faster and exact.
                let set = kind != ControlType::Document
                    && element
                        .get_pattern::<UIValuePattern>()
                        .ok()
                        .filter(|pattern| !pattern.is_readonly().unwrap_or(true))
                        .is_some_and(|pattern| pattern.set_value(text).is_ok());
                if !set {
                    element.set_focus()?;
                    // Faster intervals drop characters in some apps (Notepad lost letters at 5 ms).
                    Keyboard::new().interval(20).send_text(text)?;
                }
            }
            "toggle" => element.get_pattern::<UITogglePattern>()?.toggle()?,
            "select" => element.get_pattern::<UISelectionItemPattern>()?.select()?,
            "expand" => element.get_pattern::<UIExpandCollapsePattern>()?.expand()?,
            "collapse" => element
                .get_pattern::<UIExpandCollapsePattern>()?
                .collapse()?,
            "scroll_into_view" => element
                .get_pattern::<UIScrollItemPattern>()?
                .scroll_into_view()?,
            "focus" => element.set_focus()?,
            _ => bail!("Unknown action {action:?}"),
        }
        self.audit(&window, &label, action);
        std::thread::sleep(SETTLE);
        Ok(format!("{action} {label}: done\n{}", self.after(window_id)))
    }

    /// Sends keys to a window (the foreground one when None). Syntax: plain text types literally;
    /// `{ctrl}s`, `{alt}{f4}`, `{enter}`, `{tab 3}`; `{ctrl}(ab)` holds Ctrl for a and b.
    pub fn key(&mut self, keys: &str, window: Option<&str>) -> Result<String> {
        stopped()?;
        let window = self.window(window)?;
        self.blocked(&window)?;
        let window_id = hwnd(&window);
        bring_forward(&window);
        let focused = self.automation.get_focused_element().ok();
        if focused.is_some_and(|element| element.is_password().unwrap_or(false))
            && !self.settings.type_into_passwords
        {
            bail!(
                "The focused field is a password field, and typing into password fields is off in desktop-control settings."
            );
        }
        Keyboard::new().interval(5).send_keys(keys)?;
        self.audit(&window, "keys", "key");
        std::thread::sleep(SETTLE);
        Ok(format!("sent keys\n{}", self.after(window_id)))
    }

    /// Brings a window to the front.
    pub fn focus(&mut self, window: &str) -> Result<String> {
        stopped()?;
        let window = self.window(Some(window))?;
        self.blocked(&window)?;
        bring_forward(&window);
        Ok(format!("focused {}", header(&window)))
    }

    /// Opens an app, file or URL through the Windows shell (no command line is parsed), then waits
    /// for a new foreground window.
    pub fn launch(&mut self, app: &str) -> Result<String> {
        stopped()?;
        if let Some(entry) = self.blocked_entry(app, "") {
            bail!("{entry} is on the blocked apps list in desktop-control settings.");
        }
        let before = unsafe { GetForegroundWindow() };
        let result = unsafe {
            ShellExecuteW(
                None,
                &HSTRING::from("open"),
                &HSTRING::from(app),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize <= 32 {
            bail!(
                "Windows could not open {app:?} (code {})",
                result.0 as isize
            );
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            let now = unsafe { GetForegroundWindow() };
            if now != before && !now.is_invalid() {
                std::thread::sleep(SETTLE);
                let window = self.automation.element_from_handle(Handle::from(now))?;
                return Ok(format!("opened {}", header(&window)));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(format!(
            "asked Windows to open {app:?}; no new window came to the front within 8 seconds"
        ))
    }

    /// Reads a window's text by OCR, for apps whose UI Automation tree is empty (games, canvases,
    /// video, remote desktops). Each line ends with the screen point at its centre.
    pub fn ocr(&self, window: Option<&str>) -> Result<String> {
        let window = self.window(window)?;
        self.blocked(&window)?;
        let lines = capture(hwnd(&window))?.ocr()?;
        Ok(format!("{} (OCR)\n{}", header(&window), lines.join("\n")))
    }

    /// A JPEG of a window, the last resort for genuinely visual questions.
    pub fn screenshot(&self, window: Option<&str>) -> Result<(String, Vec<u8>)> {
        let window = self.window(window)?;
        self.blocked(&window)?;
        Ok((header(&window), capture(hwnd(&window))?.jpeg()?))
    }

    /// Clicks a screen point, for targets only OCR found. Prefer `act` on an element ID.
    pub fn click_at(&mut self, x: i32, y: i32, button: &str) -> Result<String> {
        stopped()?;
        let point = Point::new(x, y);
        let element = self.automation.element_from_point(point)?;
        if let Some(entry) =
            self.blocked_entry(&element.get_name().unwrap_or_default(), &process(&element))
        {
            bail!("{entry} is on the blocked apps list in desktop-control settings.");
        }
        let mouse = Mouse::default();
        match button {
            "right" => mouse.right_click(&point)?,
            "double" => mouse.double_click(&point)?,
            _ => mouse.click(&point)?,
        }
        self.audit(&element, &format!("point {x},{y}"), "click_at");
        std::thread::sleep(SETTLE);
        Ok(format!(
            "clicked {x},{y}\n{}",
            self.after(unsafe { GetForegroundWindow() }.0 as isize)
        ))
    }

    /// Polls until an element matching `text` appears, instead of a fixed sleep.
    pub fn wait_for(
        &mut self,
        text: &str,
        window: Option<&str>,
        timeout: Duration,
    ) -> Result<String> {
        let deadline = Instant::now() + timeout;
        loop {
            let found = self.find(text, None, window)?;
            if !found.starts_with("No element matching") {
                return Ok(found);
            }
            if Instant::now() >= deadline {
                bail!("{text:?} did not appear within {} ms", timeout.as_millis());
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    fn scan(
        &mut self,
        window: &UIElement,
        hwnd: isize,
        filter: Option<(String, Option<String>)>,
        limit: usize,
    ) -> Scan {
        let mut scan = Scan {
            lines: Vec::new(),
            limit,
            truncated: false,
            filter,
        };
        match window.build_updated_cache(&self.cache) {
            Ok(root) => self.visit(&root, 0, "", hwnd, &mut scan),
            Err(error) => scan
                .lines
                .push((0, format!("(could not read this window: {error})"))),
        }
        scan
    }

    fn visit(
        &mut self,
        parent: &UIElement,
        depth: usize,
        parent_name: &str,
        hwnd: isize,
        scan: &mut Scan,
    ) {
        let children = parent.get_cached_children().unwrap_or_default();
        let mut shown = 0;
        for (index, child) in children.iter().enumerate() {
            if scan.lines.len() >= scan.limit {
                scan.truncated = true;
                return;
            }
            if shown == SIBLINGS && scan.filter.is_none() {
                scan.lines
                    .push((depth, format!("… {} more", children.len() - index)));
                return;
            }
            if child.is_cached_offscreen().unwrap_or(false) {
                continue;
            }
            let (next, name) = match self.line_for(child, hwnd, parent_name, scan.filter.as_ref()) {
                Some((line, name)) => {
                    shown += 1;
                    scan.lines.push((depth, line));
                    (depth + 1, name)
                }
                None => (depth, parent_name.to_string()),
            };
            self.visit(
                child,
                if scan.filter.is_some() { 0 } else { next },
                &name,
                hwnd,
                scan,
            );
        }
    }

    /// One line (and the element's name) for an element, or None when it carries nothing worth
    /// showing. Unnamed layout containers and text that repeats its parent's name are skipped, but
    /// their children still print.
    fn line_for(
        &mut self,
        element: &UIElement,
        hwnd: isize,
        parent_name: &str,
        filter: Option<&(String, Option<String>)>,
    ) -> Option<(String, String)> {
        let kind = element.get_cached_control_type().ok()?;
        let role = format!("{kind:?}").to_lowercase();
        let name = clean(&element.get_cached_name().unwrap_or_default());
        let password = element.is_cached_password().unwrap_or(false);
        let states = states(element, kind, &name, password);
        let interactive = matches!(
            kind,
            ControlType::Button
                | ControlType::Edit
                | ControlType::CheckBox
                | ControlType::ComboBox
                | ControlType::ListItem
                | ControlType::MenuItem
                | ControlType::TabItem
                | ControlType::RadioButton
                | ControlType::Hyperlink
                | ControlType::Slider
                | ControlType::TreeItem
                | ControlType::SplitButton
                | ControlType::Spinner
                | ControlType::DataItem
                | ControlType::Document
        );
        if (name.is_empty() && states.is_empty() && !interactive)
            || (kind == ControlType::Text && parent_name.starts_with(&name))
        {
            return None;
        }
        if let Some((text, wanted)) = filter {
            let haystack = format!(
                "{} {}",
                name.to_lowercase(),
                states.join(" ").to_lowercase()
            );
            if !haystack.contains(text.as_str())
                || wanted.as_ref().is_some_and(|wanted| *wanted != role)
            {
                return None;
            }
        }
        let id = self.id_for(element);
        let label = format!("{role} {name:?}");
        let line = [format!("[{id}] {label}")]
            .into_iter()
            .chain(states)
            .collect::<Vec<_>>()
            .join(" ");
        self.targets.insert(
            id,
            Target {
                element: element.clone(),
                kind,
                window: hwnd,
                label,
                password,
            },
        );
        Some((line, name))
    }

    fn id_for(&mut self, element: &UIElement) -> String {
        let key = element.get_runtime_id().unwrap_or_default();
        if let Some(id) = self.ids.get(&key).filter(|_| !key.is_empty()) {
            return id.clone();
        }
        self.next += 1;
        let id = format!("e{}", self.next);
        if !key.is_empty() {
            self.ids.insert(key, id.clone());
        }
        id
    }

    /// What changed in the acted-on window, plus the new foreground window when it moved.
    fn after(&mut self, window_id: isize) -> String {
        let changes = self
            .snapshot(Some(&format!("w{window_id:x}")), true)
            .unwrap_or_else(|error| format!("(the window is gone or unreadable: {error})"));
        let foreground = unsafe { GetForegroundWindow() }.0 as isize;
        if foreground == window_id || foreground == 0 {
            return changes;
        }
        let now = self
            .automation
            .element_from_handle(Handle::from(foreground))
            .map(|window| header(&window))
            .unwrap_or_default();
        format!("{changes}\nforeground is now [w{foreground:x}] {now}")
    }

    fn top_level(&self) -> Result<Vec<UIElement>> {
        let root = self.automation.get_root_element()?;
        Ok(
            std::iter::successors(self.walker.get_first_child(&root).ok(), |element| {
                self.walker.get_next_sibling(element).ok()
            })
            .collect(),
        )
    }

    /// Resolves `w<hex handle>`, a title fragment or a process name; None means the foreground window.
    fn window(&self, query: Option<&str>) -> Result<UIElement> {
        let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) else {
            return Ok(self
                .automation
                .element_from_handle(Handle::from(unsafe { GetForegroundWindow() }))?);
        };
        if let Some(handle) = query
            .strip_prefix('w')
            .and_then(|hex| isize::from_str_radix(hex, 16).ok())
        {
            return Ok(self.automation.element_from_handle(Handle::from(handle))?);
        }
        let wanted = query.to_lowercase();
        self.top_level()?
            .into_iter()
            .find(|window| {
                window
                    .get_name()
                    .is_ok_and(|title| title.to_lowercase().contains(&wanted))
                    || process(window).to_lowercase().trim_end_matches(".exe")
                        == wanted.trim_end_matches(".exe")
            })
            .ok_or_else(|| anyhow!("No window matches {query:?}. Use windows to list them."))
    }

    fn blocked(&self, window: &UIElement) -> Result<()> {
        match self.blocked_entry(&window.get_name().unwrap_or_default(), &process(window)) {
            Some(entry) => {
                bail!("{entry} is on the blocked apps list in desktop-control settings.")
            }
            None => Ok(()),
        }
    }

    fn blocked_entry(&self, title: &str, process: &str) -> Option<String> {
        let title = title.to_lowercase();
        let process = process.to_lowercase();
        self.settings
            .blocked_apps
            .iter()
            .find(|entry| {
                let entry = entry.to_lowercase();
                !entry.is_empty()
                    && (title.contains(&entry)
                        || process.trim_end_matches(".exe") == entry.trim_end_matches(".exe"))
            })
            .cloned()
    }

    fn audit(&self, window: &UIElement, label: &str, action: &str) {
        let Some(path) = &self.settings.audit_log else {
            return;
        };
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|time| time.as_secs())
            .unwrap_or_default();
        let line = format!("{seconds}\t{}\t{label}\t{action}\n", process(window));
        // The audit log is best effort: a full disk must not block the user's own control session.
        let _ = path.parent().map(std::fs::create_dir_all);
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| file.write_all(line.as_bytes()));
    }
}

/// Registers Ctrl+Alt+Esc system-wide. Pressing it stops all further actions in this process.
pub fn start_stop_hotkey() {
    std::thread::spawn(|| {
        if let Err(error) = unsafe {
            RegisterHotKey(
                None,
                1,
                MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                VK_ESCAPE.0 as u32,
            )
        } {
            eprintln!("cryzo-desktop: could not register the Ctrl+Alt+Esc stop hotkey: {error}");
            return;
        }
        let mut message = MSG::default();
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
            if message.message == WM_HOTKEY {
                STOPPED.store(true, Ordering::SeqCst);
                eprintln!("cryzo-desktop: stopped by Ctrl+Alt+Esc");
            }
        }
    });
}

fn stopped() -> Result<()> {
    if STOPPED.load(Ordering::SeqCst) {
        bail!("Desktop control was stopped with Ctrl+Alt+Esc. Restart it to continue.");
    }
    Ok(())
}

fn states(element: &UIElement, kind: ControlType, name: &str, password: bool) -> Vec<String> {
    // Roles that carry no useful state (a link's value is its URL, which costs many tokens and
    // rarely helps).
    if matches!(
        kind,
        ControlType::Hyperlink
            | ControlType::Text
            | ControlType::Pane
            | ControlType::Group
            | ControlType::Image
            | ControlType::Separator
            | ControlType::Window
            | ControlType::TitleBar
            | ControlType::ScrollBar
            | ControlType::Thumb
            | ControlType::Header
            | ControlType::ToolBar
            | ControlType::StatusBar
    ) {
        return Vec::new();
    }
    let property = |property| element.get_cached_property_value(property).ok();
    let available = |flag| {
        property(flag)
            .and_then(|value| value.try_into().ok())
            .unwrap_or(false)
    };
    let number = |flag, state| {
        available(flag)
            .then(|| property(state))
            .flatten()
            .and_then(|value| TryInto::<i32>::try_into(value).ok())
    };
    let value = available(UIProperty::IsValuePatternAvailable)
        .then(|| property(UIProperty::ValueValue))
        .flatten()
        .and_then(|value| value.get_string().ok())
        .filter(|value| !value.is_empty() && value != name);
    let toggle = number(
        UIProperty::IsTogglePatternAvailable,
        UIProperty::ToggleToggleState,
    )
    .and_then(|state| match state {
        0 => Some("unchecked"),
        1 => Some("checked"),
        2 => Some("mixed"),
        _ => None,
    });
    let selected = (available(UIProperty::IsSelectionItemPatternAvailable)
        && property(UIProperty::SelectionItemIsSelected)
            .and_then(|value| value.try_into().ok())
            .unwrap_or(false))
    .then_some("selected");
    let expanded = number(
        UIProperty::IsExpandCollapsePatternAvailable,
        UIProperty::ExpandCollapseExpandCollapseState,
    )
    .and_then(|state| match state {
        0 => Some("collapsed"),
        1 | 2 => Some("expanded"),
        _ => None,
    });
    let disabled = (!element.is_cached_enabled().unwrap_or(true)).then_some("disabled");
    let focused = element
        .has_cached_keyboard_focus()
        .unwrap_or(false)
        .then_some("focused");
    value
        .map(|value| {
            if password {
                "value=hidden".to_string()
            } else {
                format!("value={:?}", clip(&clean(&value), 80))
            }
        })
        .into_iter()
        .chain(
            [toggle, selected, expanded, disabled, focused]
                .into_iter()
                .flatten()
                .map(String::from),
        )
        .collect()
}

fn changes(header: &str, previous: &[String], now: &[String]) -> String {
    let before = previous
        .iter()
        .map(|line| line.trim())
        .collect::<HashSet<_>>();
    let after = now.iter().map(|line| line.trim()).collect::<HashSet<_>>();
    let removed = previous
        .iter()
        .map(|line| line.trim())
        .filter(|line| !after.contains(line))
        .map(|line| format!("- {line}"));
    let added = now
        .iter()
        .map(|line| line.trim())
        .filter(|line| !before.contains(line))
        .map(|line| format!("+ {line}"));
    let lines = removed.chain(added).collect::<Vec<_>>();
    if lines.is_empty() {
        return format!("{header}: no visible change");
    }
    format!("{header} changed:\n{}", lines.join("\n"))
}

fn header(window: &UIElement) -> String {
    let title = clean(&window.get_name().unwrap_or_default());
    let size = window
        .get_bounding_rectangle()
        .map(|rect| format!(" {}x{}", rect.get_width(), rect.get_height()))
        .unwrap_or_default();
    format!(
        "window [w{:x}] {title:?} ({}){size}",
        hwnd(window),
        process(window)
    )
}

fn bring_forward(window: &UIElement) {
    // Either call alone can be refused by Windows' foreground rules; together they usually work.
    let _ = window.set_focus();
    let _ = unsafe { SetForegroundWindow(HWND(hwnd(window) as *mut _)) };
}

fn hwnd(element: &UIElement) -> isize {
    element
        .get_native_window_handle()
        .map(Into::into)
        .unwrap_or_default()
}

fn process(element: &UIElement) -> String {
    let Ok(pid) = element.get_process_id() else {
        return String::new();
    };
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return String::new();
    };
    let mut buffer = [0u16; 1024];
    let mut size = buffer.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    }
    .is_ok();
    let _ = unsafe { CloseHandle(handle) };
    if !ok {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buffer[..size as usize]);
    path.rsplit('\\').next().unwrap_or_default().to_string()
}

fn clean(text: &str) -> String {
    // Icon fonts put glyphs in the Unicode private use area; they carry no meaning as text.
    let text = text
        .chars()
        .filter(|c| !('\u{e000}'..='\u{f8ff}').contains(c))
        .collect::<String>();
    clip(&text.split_whitespace().collect::<Vec<_>>().join(" "), 120)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max).collect::<String>())
}
