use std::{
    cell::OnceCell,
    collections::{HashMap, HashSet},
    io::Write,
    sync::atomic::Ordering,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) use crate::ax::quote;
use crate::common::{OUTCOME_WAIT, outcome_matches, outcome_note, outcome_query};
use crate::common::{STOPPED, line_id, summary};
pub(crate) use crate::common::{changes, clean, clip, is_stopped, stop, stopped, verb};
use anyhow::{Result, anyhow, bail, ensure};
use uiautomation::{
    UIAutomation, UIElement, UITreeWalker,
    core::UICacheRequest,
    inputs::{Mouse, Parser},
    patterns::{
        UIExpandCollapsePattern, UIInvokePattern, UIScrollItemPattern, UIScrollPattern,
        UISelectionItemPattern, UITextPattern, UITogglePattern, UIValuePattern,
    },
    types::{ControlType, Handle, Point, ScrollAmount, TreeScope, UIProperty},
    variants::Variant,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, RECT},
        System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::Accessibility::IUIAutomation2,
        UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
        UI::{
            Input::KeyboardAndMouse::{
                INPUT, KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey,
                SendInput, VK_ESCAPE,
            },
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                GetForegroundWindow, GetMessageW, IsWindow, MSG, SW_SHOWNORMAL,
                SetForegroundWindow, WM_HOTKEY,
            },
        },
    },
    core::{HSTRING, Interface, PCWSTR, PWSTR},
};

use serde_json::Value;

use crate::{
    Output, Settings,
    capture::capture,
    overlay::{CursorPreview, Overlay},
};

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
/// Settle time after a launched window comes to the front.
const SETTLE: Duration = Duration::from_millis(350);
/// After an action, the window is re-read every `POLL` until it has not changed for `QUIET`, so
/// content that loads a moment later (a fetched list, a dialog) lands in the action's own result.
const POLL: Duration = Duration::from_millis(50);
const QUIET: Duration = Duration::from_millis(350);
const SETTLE_LIMIT: Duration = Duration::from_secs(3);

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
    /// The Cryzo Chrome, connected on the first browser tool call.
    browser: Option<crate::browser::Browser>,
    overlay: OnceCell<Option<Overlay>>,
}

struct Target {
    element: UIElement,
    window: isize,
    name: String,
    /// Lowercase control type, such as `button` or `tabitem`.
    role: String,
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
        // An app that stops answering (File Explorer did, mid-rename) would otherwise hold every
        // later call, since UI Automation waits for it and `windows` asks every window, Explorer's
        // taskbar included. With these, a call to it fails within seconds instead.
        if let Ok(automation) = automation.as_ref().cast::<IUIAutomation2>() {
            unsafe {
                let _ = automation.SetConnectionTimeout(2_000);
                let _ = automation.SetTransactionTimeout(8_000);
            }
        }
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
            browser: None,
            overlay: OnceCell::new(),
        })
    }

    /// Applies settings the user changed while this desktop was running.
    pub fn configure(&mut self, settings: Settings) {
        self.settings = settings;
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
        if let Some(overlay) = self.running_overlay() {
            match command {
                "windows" | "snapshot" | "find" | "wait_for" | "ocr" | "screenshot" => {
                    overlay.step("Reading the screen")
                }
                "browser_snapshot" | "browser_find" | "browser_downloads" | "browser_tabs"
                | "browser_capture" => overlay.step("Reading the page"),
                _ => {}
            }
        }
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
                let action = required("action")?;
                self.act(&id, action, text("value"), outcome_query(args, action)?)
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
                let (header, jpeg) = self.screenshot(text("window"))?;
                return Ok(Output::Image { header, jpeg });
            }
            command if command.starts_with("browser_") => {
                let (max_lines, passwords) =
                    (self.settings.max_lines, self.settings.type_into_passwords);
                let overlay = if crate::acts(command) {
                    self.overlay().cloned()
                } else {
                    None
                };
                self.browser()?
                    .call(command, args, max_lines, passwords, overlay.as_ref())
            }
            _ => Err(anyhow!("Unknown desktop tool {command:?}")),
        };
        output.map(Output::Text)
    }

    /// The Cryzo Chrome, started or reconnected when it was closed.
    fn browser(&mut self) -> Result<&mut crate::browser::Browser> {
        if !self.browser.as_mut().is_some_and(|browser| browser.alive()) {
            self.browser = Some(crate::browser::Browser::connect()?);
        }
        self.browser
            .as_mut()
            .ok_or_else(|| anyhow!("the browser is not connected"))
    }

    /// A human sentence for an approval card, such as `click button "Post now" in chrome.exe`,
    /// with a warning when the app reaches far beyond itself (a shell, every file, system
    /// settings), so the user can decide whether the task warrants that.
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
            "act" if text("id").is_empty() => format!(
                "{} {} {:?} in {}{}",
                text("action"),
                args.get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("element"),
                text("name"),
                window("window"),
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

    /// The process a command would act on, for "ask once per app". Browser commands count each
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
            "act" if text("id").is_none() => process(&self.window(text("window")).ok()?),
            // A batch counts as one app only when every step acts on that same app.
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
        let top = self.top_level()?;
        let lines = top
            .iter()
            .chain(self.dialogs(&top).iter())
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
        let first = self.read(&window, hwnd);
        // Chromium and WebView2 build their web accessibility tree only after the first UIA
        // client asks, so a near-empty first read, or a page document with nothing in it yet
        // (Chrome's toolbar alone is more than a few lines), is usually just a cold tree.
        let (lines, truncated) = if first.0.len() < 8 || cold_document(&first.0) {
            std::thread::sleep(Duration::from_millis(700));
            self.read(&window, hwnd)
        } else {
            first
        };
        let header = header(&window);
        let previous = self.last.insert(hwnd, lines.clone());
        let ocr = if sparse(&lines) {
            ocr(hwnd)
        } else {
            String::new()
        };
        if let (true, Some(previous)) = (diff, previous) {
            return Ok(format!("{}{ocr}", changes(&header, &previous, &lines)));
        }
        let note = if truncated {
            "\n  … cut off at the line or time budget; use find to reach the rest"
        } else {
            ""
        };
        Ok(format!("{header}\n{}{note}{ocr}", lines.join("\n")))
    }

    /// Elements whose name or value contains `text` (and whose role matches, when given).
    pub fn find(&mut self, text: &str, role: Option<&str>, window: Option<&str>) -> Result<String> {
        let window = self.window(window)?;
        self.blocked(&window)?;
        let filter = (text.to_lowercase(), role.map(str::to_lowercase));
        let scan = self.search(&window, hwnd(&window), filter);
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

    /// The ID of the element called `name`: an exact (case-insensitive) name match first, else the
    /// first element whose name or value contains it. Saves a find before every act.
    fn resolve(&mut self, name: &str, role: Option<&str>, window: Option<&str>) -> Result<String> {
        let window = self.window(window)?;
        self.blocked(&window)?;
        let hwnd = hwnd(&window);
        let wanted = name.to_lowercase();
        let role = role.map(str::to_lowercase);
        let exact = |target: &Target| {
            target.name.to_lowercase() == wanted
                && role.as_ref().is_none_or(|role| *role == target.role)
        };
        // The window's last settled read usually holds the element already, which saves a full
        // scan (200 ms to 1.5 s in WebView2). A live name read confirms it still exists.
        let cached = self
            .last
            .get(&hwnd)
            .into_iter()
            .flatten()
            .filter_map(|line| line_id(line.trim()))
            .find(|id| {
                self.targets
                    .get(id.as_str())
                    .is_some_and(|target| exact(target) && target.element.get_name().is_ok())
            });
        if let Some(id) = cached {
            return Ok(id);
        }
        let scan = self.search(&window, hwnd, (wanted.clone(), role.clone()));
        let ids = scan
            .lines
            .iter()
            .filter_map(|(_, line)| line_id(line))
            .collect::<Vec<_>>();
        ids.iter()
            .find(|id| self.targets.get(id.as_str()).is_some_and(exact))
            .or(ids.first())
            .cloned()
            .ok_or_else(|| anyhow!("No element matching {name:?} in {}", header(&window)))
    }

    /// Runs several tools in order in one call and returns each result. Stops at the first failure.
    /// Acting steps before the last report one summary line each, since the model needs the end
    /// state, not every view in between; reading steps and the last step report in full.
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

    /// Acts on an element from a snapshot, then returns what changed in its window.
    pub fn act(
        &mut self,
        id: &str,
        action: &str,
        value: Option<&str>,
        wait_for: Option<&str>,
    ) -> Result<String> {
        let started = Instant::now();
        stopped()?;
        let target = self
            .targets
            .get(id)
            .ok_or_else(|| anyhow!("Unknown element {id}. Take a new snapshot."))?;
        let element = target.element.clone();
        let label = format!("[{id}] {}", target.label);
        let password = target.password;
        let window_id = target.window;
        let window = self
            .automation
            .element_from_handle(Handle::from(window_id))?;
        self.blocked(&window)?;
        if action == "read" {
            ensure!(
                !password,
                "{label} is a password field; its text is never read."
            );
            return Ok(format!("{label} text:\n{}", full_text(&element)));
        }
        let step = format!("{} {}", verb(action), target.label);
        let outcome = wait_for.map(|query| {
            let (lines, truncated) = self.read(&window, window_id);
            (query, outcome_matches(&lines, truncated, query) == Some(0))
        });
        let prepared = Instant::now();
        let preview = self.show(&step, Some(&element));
        let presented = Instant::now();
        // Semantic patterns address their UIA element directly. Physical input still previews
        // its target before delivery, including when a semantic pattern cannot take the action.
        let before_input = || {
            preview.wait();
            stopped()
        };
        stopped()?;
        match action {
            "click" => match element.get_pattern::<UIInvokePattern>() {
                Ok(pattern) => pattern.invoke()?,
                Err(_) => {
                    before_input()?;
                    element.click()?;
                }
            },
            "double_click" => {
                before_input()?;
                element.double_click()?;
            }
            "right_click" => {
                before_input()?;
                element.right_click()?;
            }
            "type" => {
                let text = value.ok_or_else(|| anyhow!("type needs a value"))?;
                if password && !self.settings.type_into_passwords {
                    bail!(
                        "Typing into password fields is off in desktop-control settings (typeIntoPasswords)."
                    );
                }
                // Setting the value is exact and instant. Injected keystrokes are not: Windows 11
                // Notepad dropped and repeated characters even at 20 ms apart. Keystrokes remain
                // the fallback for controls without a writable value.
                let field = element.get_pattern::<UIValuePattern>().ok();
                // A read-only value, such as a file's Name column in Explorer, can't take text,
                // and forcing focus onto it in the middle of a rename stopped Explorer answering.
                if field
                    .as_ref()
                    .is_some_and(|pattern| pattern.is_readonly().unwrap_or(false))
                {
                    bail!("{label} is read-only.{}", self.editing());
                }
                // A cell in a list row, such as a file's Name in Explorer, takes text only while
                // it's being edited. Setting it otherwise renames the item outright, and during
                // another rename Explorer stopped answering until it gave up a minute later.
                let cell = self.walker.get_parent(&element).is_ok_and(|parent| {
                    matches!(
                        parent.get_control_type(),
                        Ok(ControlType::ListItem | ControlType::DataItem)
                    )
                });
                if cell && !element.has_keyboard_focus().unwrap_or(false) {
                    bail!(
                        "{label} is a cell of a list row; setting it would rename the item without editing it.{} To rename an item, select it and press F2 first.",
                        self.editing()
                    );
                }
                stopped()?;
                let set = field.is_some_and(|pattern| pattern.set_value(text).is_ok());
                if !set {
                    before_input()?;
                    element.set_focus()?;
                    type_text(text, None)?;
                }
            }
            "toggle" => element.get_pattern::<UITogglePattern>()?.toggle()?,
            // Many list items only respond to a click (Windows Settings pages, for one).
            "select" => match element.get_pattern::<UISelectionItemPattern>() {
                Ok(pattern) => pattern.select()?,
                Err(_) => match element.get_pattern::<UIInvokePattern>() {
                    Ok(pattern) => pattern.invoke()?,
                    Err(_) => {
                        before_input()?;
                        element.click()?;
                    }
                },
            },
            // Like Ctrl+click: keeps what is already selected, for picking several files.
            "add_to_selection" => element
                .get_pattern::<UISelectionItemPattern>()?
                .add_to_selection()?,
            "expand" => element.get_pattern::<UIExpandCollapsePattern>()?.expand()?,
            "collapse" => element
                .get_pattern::<UIExpandCollapsePattern>()?
                .collapse()?,
            "scroll_into_view" => element
                .get_pattern::<UIScrollItemPattern>()?
                .scroll_into_view()?,
            "scroll_down" | "scroll_up" => self
                .scroller(&element)
                .ok_or_else(|| anyhow!("Nothing scrollable contains {label}."))?
                .scroll(
                    ScrollAmount::NoAmount,
                    if action == "scroll_down" {
                        ScrollAmount::LargeIncrement
                    } else {
                        ScrollAmount::LargeDecrement
                    },
                )?,
            "focus" => element.set_focus()?,
            _ => bail!("Unknown action {action:?}"),
        }
        if matches!(
            action,
            "click" | "double_click" | "right_click" | "toggle" | "select"
        ) && let Some(overlay) = self.running_overlay()
        {
            overlay.click();
        }
        let delivered = Instant::now();
        self.audit(&window, &label, action);
        let observing = Instant::now();
        let observation = self.after_action(window_id, outcome);
        if std::env::var_os("CRYZO_DESKTOP_PROFILE_SETTLE").is_some() {
            eprintln!(
                "desktop action prepare_ms={} preview_ms={} delivery_ms={} audit_ms={} observation_ms={} total_ms={}",
                (prepared - started).as_millis(),
                (presented - prepared).as_millis(),
                (delivered - presented).as_millis(),
                (observing - delivered).as_millis(),
                observing.elapsed().as_millis(),
                started.elapsed().as_millis(),
            );
        }
        Ok(format!("{action} {label}: done\n{observation}"))
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
        if focused
            .as_ref()
            .is_some_and(|element| element.is_password().unwrap_or(false))
            && !self.settings.type_into_passwords
        {
            bail!(
                "The focused field is a password field, and typing into password fields is off in desktop-control settings."
            );
        }
        let plain = !keys.contains(['{', '}', '(', ')', '\n', '\t']);
        let title = clean(&window.get_name().unwrap_or_default());
        let step = if plain {
            format!("Typing in {title}")
        } else {
            format!("Pressing {} in {title}", clip(keys, 30))
        };
        self.show(&step, focused.as_ref()).wait();
        stopped()?;
        // Plain text into an empty field is set exactly instead of typed. Otherwise it is typed,
        // and the field is read back so lost keystrokes are reported instead of passing silently.
        let field = focused
            .and_then(|element| element.get_pattern::<UIValuePattern>().ok())
            .filter(|pattern| !pattern.is_readonly().unwrap_or(true));
        let set = plain
            && field.as_ref().is_some_and(|pattern| {
                pattern.get_value().is_ok_and(|value| value.is_empty())
                    && pattern.set_value(keys).is_ok()
            });
        if set {
            // Setting the value can leave it selected; typing next would replace it.
            send_keys("{ctrl}{end}", None)?;
        } else {
            send_keys(keys, field.as_ref())?;
        }
        // The last keys can still be on their way when the loop ends.
        let deadline = Instant::now() + Duration::from_millis(500);
        let note = field
            .filter(|_| plain)
            .and_then(|pattern| loop {
                let value = pattern.get_value().ok()?;
                if value.contains(keys) || Instant::now() >= deadline {
                    break Some(value);
                }
                std::thread::sleep(Duration::from_millis(25));
            })
            .filter(|value| !value.contains(keys))
            .map(|value| {
                let tail = value.chars().rev().take(60).collect::<Vec<_>>();
                format!(
                    "\n(warning: some keystrokes were lost; the field ends with {}. Set the whole text with act type.)",
                    quote(&tail.into_iter().rev().collect::<String>())
                )
            })
            .unwrap_or_default();
        self.audit(&window, "keys", "key");
        Ok(format!("sent keys\n{}{note}", self.after(window_id)))
    }

    /// Brings a window to the front.
    pub fn focus(&mut self, window: &str) -> Result<String> {
        stopped()?;
        let window = self.window(Some(window))?;
        self.blocked(&window)?;
        let step = format!(
            "Switching to {}",
            clean(&window.get_name().unwrap_or_default())
        );
        self.show(&step, None);
        bring_forward(&window);
        Ok(format!("focused {}", header(&window)))
    }

    /// Shows the step on the overlay, gliding the agent cursor to `element` when it has a place
    /// on screen.
    fn show(&self, step: &str, element: Option<&UIElement>) -> CursorPreview {
        let Some(overlay) = self.overlay() else {
            return CursorPreview::default();
        };
        let rect = element
            .and_then(|element| element.get_bounding_rectangle().ok())
            .map(|rect| RECT {
                left: rect.get_left(),
                top: rect.get_top(),
                right: rect.get_right(),
                bottom: rect.get_bottom(),
            })
            .filter(|rect| rect.right > rect.left && rect.bottom > rect.top);
        match rect {
            Some(rect) => overlay.preview(step, rect.into()),
            None => {
                overlay.step(step);
                CursorPreview::default()
            }
        }
    }

    /// Opens an app, file or URL through the Windows shell (no command line is parsed), then waits
    /// for a new foreground window.
    pub fn launch(&mut self, app: &str) -> Result<String> {
        stopped()?;
        if let Some(entry) = self.blocked_entry(app, "") {
            bail!("{entry} is on the blocked apps list in desktop-control settings.");
        }
        self.show(&format!("Opening {}", clip(app, 60)), None);
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
        if let Some(overlay) = self.overlay() {
            let spot = RECT {
                left: x - 10,
                top: y - 10,
                right: x + 10,
                bottom: y + 10,
            };
            overlay.point(format!("Clicking at {x}, {y}"), spot.into());
            stopped()?;
            overlay.click();
        }
        let mouse = Mouse::default();
        match button {
            "right" => mouse.right_click(&point)?,
            "double" => mouse.double_click(&point)?,
            _ => mouse.click(&point)?,
        }
        self.audit(&element, &format!("point {x},{y}"), "click_at");
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
            // A plain scan, not find: paging a list on every poll would scroll it back and forth.
            let target = self.window(window)?;
            self.blocked(&target)?;
            let scan = self.scan(
                &target,
                hwnd(&target),
                Some((text.to_lowercase(), None)),
                50,
            );
            if !scan.lines.is_empty() {
                return Ok(scan
                    .lines
                    .into_iter()
                    .map(|(_, line)| line)
                    .collect::<Vec<_>>()
                    .join("\n"));
            }
            if Instant::now() >= deadline {
                // Once, at the end: a virtualized list may hold the match out of view.
                let found = self.search(&target, hwnd(&target), (text.to_lowercase(), None));
                if !found.lines.is_empty() {
                    return Ok(found
                        .lines
                        .into_iter()
                        .map(|(_, line)| line)
                        .collect::<Vec<_>>()
                        .join("\n"));
                }
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

    /// Matching elements, paging through the window's scrollable areas when none are loaded yet.
    /// Windows Settings and other virtualized lists only create items once they scroll into view,
    /// so a plain scan cannot see them.
    fn search(
        &mut self,
        window: &UIElement,
        hwnd: isize,
        filter: (String, Option<String>),
    ) -> Scan {
        let scan = self.scan(window, hwnd, Some(filter.clone()), 50);
        if !scan.lines.is_empty() {
            return scan;
        }
        let areas = self
            .automation
            .create_property_condition(
                UIProperty::IsScrollPatternAvailable,
                Variant::from(true),
                None,
            )
            .and_then(|condition| window.find_all(TreeScope::Descendants, &condition))
            .unwrap_or_default();
        let deadline = Instant::now() + Duration::from_secs(6);
        for area in areas {
            let Some(pattern) = area
                .get_pattern::<UIScrollPattern>()
                .ok()
                .filter(|pattern| pattern.is_vertically_scrollable().unwrap_or(false))
            else {
                continue;
            };
            let start = pattern.get_vertical_scroll_percent().unwrap_or(0.0);
            // From the top, since the match may sit above the current view. -1 leaves the
            // horizontal position alone (UIA_ScrollPatternNoScroll).
            let _ = pattern.set_scroll_percent(-1.0, 0.0);
            let mut position = -1.0;
            while Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(150));
                let found = self.scan(window, hwnd, Some(filter.clone()), 50);
                if !found.lines.is_empty() {
                    return found;
                }
                let now = pattern.get_vertical_scroll_percent().unwrap_or(100.0);
                if now >= 100.0 || now == position {
                    break;
                }
                position = now;
                if pattern
                    .scroll(ScrollAmount::NoAmount, ScrollAmount::LargeIncrement)
                    .is_err()
                {
                    break;
                }
            }
            let _ = pattern.set_scroll_percent(-1.0, start);
        }
        scan
    }

    /// The nearest vertically scrollable area that holds `element` (or the element itself), else
    /// the first one inside it, so scroll works on any element of a list or on a whole pane.
    fn scroller(&self, element: &UIElement) -> Option<UIScrollPattern> {
        let scrollable = |element: &UIElement| {
            element
                .get_pattern::<UIScrollPattern>()
                .ok()
                .filter(|pattern| pattern.is_vertically_scrollable().unwrap_or(false))
        };
        std::iter::successors(Some(element.clone()), |element| {
            self.walker.get_parent(element).ok()
        })
        .take(30)
        .find_map(|element| scrollable(&element))
        .or_else(|| {
            let condition = self
                .automation
                .create_property_condition(
                    UIProperty::IsScrollPatternAvailable,
                    Variant::from(true),
                    None,
                )
                .ok()?;
            element
                .find_all(TreeScope::Descendants, &condition)
                .ok()?
                .iter()
                .find_map(scrollable)
        })
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
        let label = format!("{role} {}", quote(&name));
        let line = [format!("[{id}] {label}")]
            .into_iter()
            .chain(states)
            .collect::<Vec<_>>()
            .join(" ");
        self.targets.insert(
            id,
            Target {
                element: element.clone(),
                window: hwnd,
                name: name.clone(),
                role,
                label,
                password,
            },
        );
        Some((line, name))
    }

    /// Names the field that has keyboard focus, when it takes text, so a refused `type` can point
    /// the model to where the text belongs, such as Explorer's rename box.
    fn editing(&self) -> String {
        let Ok(focused) = self.automation.get_focused_element() else {
            return String::new();
        };
        let writable = focused
            .get_pattern::<UIValuePattern>()
            .ok()
            .is_some_and(|pattern| !pattern.is_readonly().unwrap_or(true));
        if !writable {
            return String::new();
        }
        let role = focused
            .get_control_type()
            .map(|kind| format!("{kind:?}").to_lowercase())
            .unwrap_or_default();
        let name = clean(&focused.get_name().unwrap_or_default());
        format!(
            " The field being edited is {role} {name:?}: type into it by that name, or send keys to it with key."
        )
    }

    fn id_for(&mut self, element: &UIElement) -> String {
        // RuntimeId travels with the cached properties. Avoid a separate provider call for
        // each control on every poll when the provider supplied the cached ID.
        let cached: Option<Vec<i32>> = element
            .get_cached_property_value(UIProperty::RuntimeId)
            .and_then(|value| value.get_array())
            .and_then(|array| array.try_into())
            .ok();
        let key = cached
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| element.get_runtime_id().unwrap_or_default());
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

    /// A window's snapshot lines, indented, and whether they were cut off.
    fn read(&mut self, window: &UIElement, hwnd: isize) -> (Vec<String>, bool) {
        let scan = self.scan(window, hwnd, None, self.settings.max_lines);
        let lines = scan
            .lines
            .iter()
            .map(|(depth, line)| format!("{}{line}", "  ".repeat(depth + 1)))
            .collect();
        (lines, scan.truncated)
    }

    /// What changed in the acted-on window once it settles, plus the new foreground window when
    /// it moved.
    fn after(&mut self, window_id: isize) -> String {
        self.after_action(window_id, None)
    }

    fn after_action(&mut self, window_id: isize, outcome: Option<(&str, bool)>) -> String {
        let settled = self.settle(window_id, outcome).unwrap_or_else(|error| {
            format!(
                "(the window is unreadable: {error}){}",
                outcome_note(outcome, false)
            )
        });
        let changes = if unsafe { IsWindow(Some(HWND(window_id as *mut _))) }.as_bool() {
            settled
        } else {
            self.last.remove(&window_id);
            format!(
                "window [w{window_id:x}] closed{}",
                outcome_note(outcome, false)
            )
        };
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

    /// Re-reads the window until it stops changing, then diffs it against its last snapshot (or
    /// prints it whole when there is none).
    fn settle(&mut self, window_id: isize, outcome: Option<(&str, bool)>) -> Result<String> {
        let window = self
            .automation
            .element_from_handle(Handle::from(window_id))?;
        let started = Instant::now();
        let deadline = started + SETTLE_LIMIT;
        let profile = std::env::var_os("CRYZO_DESKTOP_PROFILE_SETTLE").is_some();
        let mut scans = 0;
        let mut scan_time = Duration::ZERO;
        let mut sleep_time = Duration::ZERO;
        let mut read = || {
            let before = Instant::now();
            let (lines, truncated) = self.read(&window, window_id);
            let duration = before.elapsed();
            scan_time += duration;
            scans += 1;
            (lines, truncated, duration)
        };
        // Read immediately, but retain the original initial observation window. A delayed
        // change resets the full quiet period. Take the last sample at the quiet deadline
        // instead of waiting an extra poll interval to check it.
        let query = outcome.and_then(|(query, fresh)| fresh.then_some(query));
        let earliest = started
            + if query.is_some() {
                OUTCOME_WAIT
            } else {
                POLL + QUIET
            };
        let (mut lines, mut truncated, mut last_scan) = read();
        let mut quiet_until = Instant::now() + QUIET;
        let mut matched = false;
        loop {
            if query.is_some_and(|query| outcome_matches(&lines, truncated, query) == Some(1)) {
                matched = true;
                break;
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let ready_at = earliest.max(quiet_until).min(deadline);
            let remaining = ready_at.saturating_duration_since(now);
            // A poll about to run into the final sample would put two full scans back to back.
            let wait = if remaining <= POLL + last_scan.min(POLL) {
                remaining
            } else {
                POLL
            };
            if !wait.is_zero() {
                std::thread::sleep(wait);
                sleep_time += now.elapsed();
            }
            let sample_at = Instant::now();
            let (next, more, duration) = read();
            truncated = more;
            last_scan = duration;
            if next != lines {
                lines = next;
                quiet_until = Instant::now() + QUIET;
            } else if sample_at >= ready_at {
                break;
            }
        }
        let header = header(&window);
        let ocr = if sparse(&lines) {
            ocr(window_id)
        } else {
            String::new()
        };
        if profile {
            eprintln!(
                "desktop settle elapsed_ms={} sleep_ms={} scan_ms={} scans={scans}",
                started.elapsed().as_millis(),
                sleep_time.as_millis(),
                scan_time.as_millis(),
            );
        }
        let update = match self.last.insert(window_id, lines.clone()) {
            Some(previous) => format!("{}{ocr}", changes(&header, &previous, &lines)),
            None => format!("{header}\n{}{ocr}", lines.join("\n")),
        };
        Ok(format!("{update}{}", outcome_note(outcome, matched)))
    }

    fn top_level(&self) -> Result<Vec<UIElement>> {
        let root = self.automation.get_root_element()?;
        Ok(
            std::iter::successors(self.walker.get_first_child(&root).ok(), |element| {
                self.walker.get_next_sibling(element).ok()
            })
            // The overlay's own windows are not something to act on, and their titles would
            // match a query like "cryzo".
            .filter(|window| {
                !window
                    .get_classname()
                    .is_ok_and(|class| class.starts_with("CryzoOverlay"))
            })
            .collect(),
        )
    }

    /// The on-screen overlay, started by the first action. None when it could not start.
    fn overlay(&self) -> Option<&Overlay> {
        self.overlay.get_or_init(Overlay::start).as_ref()
    }

    /// The overlay only when an action has already started it, so reading alone never shows it
    /// but reads during a run keep it up.
    fn running_overlay(&self) -> Option<&Overlay> {
        self.overlay.get().and_then(Option::as_ref)
    }

    /// A new user turn: a Stop from the previous turn is cleared, because sending a new message
    /// is the user letting Cryzo act again.
    pub fn begin_turn(&mut self) {
        STOPPED.store(false, Ordering::SeqCst);
        if let Some(overlay) = self.running_overlay() {
            overlay.resume();
        }
    }

    /// The turn ended: the overlay goes away now instead of fading out on its own.
    pub fn end_turn(&mut self) {
        if let Some(overlay) = self.running_overlay() {
            overlay.done();
        }
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
            if !unsafe { IsWindow(Some(HWND(handle as *mut _))) }.as_bool() {
                bail!("Window {query} has closed. Use windows to list the open ones.");
            }
            return Ok(self.automation.element_from_handle(Handle::from(handle))?);
        }
        let wanted = query.to_lowercase();
        let windows = self.top_level()?;
        let dialogs = self.dialogs(&windows);
        let title = |window: &UIElement| window.get_name().unwrap_or_default().to_lowercase();
        // An exact title or app name wins over a title that merely contains the query, so
        // "Cryzo" picks the Cryzo app, not a browser tab titled "Cryzo Desktop - App Review".
        let exact = windows.iter().find(|window| {
            title(window) == wanted
                || process(window).to_lowercase().trim_end_matches(".exe")
                    == wanted.trim_end_matches(".exe")
        });
        exact
            .or_else(|| dialogs.iter().find(|dialog| title(dialog) == wanted))
            .or_else(|| {
                windows
                    .iter()
                    .find(|window| title(window).contains(&wanted))
            })
            .or_else(|| {
                dialogs
                    .iter()
                    .find(|dialog| title(dialog).contains(&wanted))
            })
            .cloned()
            .ok_or_else(|| anyhow!("No window matches {query:?}. Use windows to list them."))
    }

    /// Dialogs such as Open, Save As or a confirmation. UI Automation lists a window's owned
    /// dialogs under that window rather than at the top level, so a lookup by title missed them.
    fn dialogs(&self, windows: &[UIElement]) -> Vec<UIElement> {
        windows
            .iter()
            .flat_map(|window| {
                std::iter::successors(self.walker.get_first_child(window).ok(), |child| {
                    self.walker.get_next_sibling(child).ok()
                })
            })
            .filter(|child| {
                child.get_control_type().ok() == Some(ControlType::Window) && hwnd(child) != 0
            })
            .collect()
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

/// Sends keys in the `key` syntax, one key press (with its modifiers) per SendInput call. When the
/// focused field exposes its text, each press waits until the text changes, so typing goes as fast
/// as the app takes it. Windows 11 Notepad lost or repeated characters under fixed pacing (5 to
/// 20 ms) and turned Unicode key events into runs of one letter even 25 ms apart.
fn send_keys(keys: &str, field: Option<&UIValuePattern>) -> Result<()> {
    let keys = keys
        .replace("\r\n", "{enter}")
        .replace('\n', "{enter}")
        .replace('\t', "{tab}");
    for (index, input) in Parser::new(false).parse_input(&keys)?.iter().enumerate() {
        if index > 0 {
            // Lets a shortcut's effect (a new tab, a dialog) appear before the next keys land.
            std::thread::sleep(Duration::from_millis(40));
        }
        for press in presses(&input.create_inputs()?) {
            // Reading a huge document back after every key would cost more than it saves.
            let before = field
                .and_then(|field| field.get_value().ok())
                .filter(|value| value.len() < 20_000);
            // Modifiers are released only once the key has landed: Notepad read Shift as already
            // up and typed \ for |.
            let release = press
                .iter()
                .rev()
                .take_while(|event| modifier(event) && up(event))
                .count();
            let (keys, release) = press.split_at(press.len() - release);
            send_input(keys)?;
            match (field, before) {
                (Some(field), Some(before)) => {
                    let deadline = Instant::now() + Duration::from_millis(200);
                    while Instant::now() < deadline
                        && field.get_value().is_ok_and(|value| value == before)
                    {
                        std::thread::sleep(Duration::from_millis(3));
                    }
                }
                _ => std::thread::sleep(Duration::from_millis(12)),
            }
            if !release.is_empty() {
                send_input(release)?;
            }
        }
    }
    Ok(())
}

/// Shift, Ctrl, Alt or a Windows key, either side.
fn modifier(event: &INPUT) -> bool {
    matches!(
        unsafe { event.Anonymous.ki }.wVk.0,
        0x10..=0x12 | 0x5b | 0x5c | 0xa0..=0xa5
    )
}

fn up(event: &INPUT) -> bool {
    unsafe { event.Anonymous.ki }.dwFlags.0 & KEYEVENTF_KEYUP.0 != 0
}

/// Splits key events into presses: each ends when a key is released with no modifier held, so a
/// shifted letter or a Ctrl shortcut stays in one piece.
fn presses(events: &[INPUT]) -> Vec<&[INPUT]> {
    let mut presses = Vec::new();
    let mut start = 0;
    let mut held = 0;
    for (index, event) in events.iter().enumerate() {
        if modifier(event) {
            held += if up(event) { -1 } else { 1 };
        }
        if up(event) && held <= 0 {
            presses.push(&events[start..=index]);
            start = index + 1;
            held = 0;
        }
    }
    if start < events.len() {
        presses.push(&events[start..]);
    }
    presses
}

/// Types `text` literally, braces and parentheses included.
fn type_text(text: &str, field: Option<&UIValuePattern>) -> Result<()> {
    let escaped = text
        .chars()
        .map(|c| match c {
            '{' | '}' | '(' | ')' => format!("{{{c}}}"),
            _ => c.to_string(),
        })
        .collect::<String>();
    send_keys(&escaped, field)
}

fn send_input(events: &[INPUT]) -> Result<()> {
    let sent = unsafe { SendInput(events, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != events.len() {
        bail!(
            "Windows accepted {sent} of {} key events (another app may be blocking input)",
            events.len()
        );
    }
    Ok(())
}

/// An element's whole text, for documents and long fields whose snapshot value is cut at 80
/// characters: the text pattern's document range, else its value, else its name.
fn full_text(element: &UIElement) -> String {
    const LIMIT: i32 = 20_000;
    let text = element
        .get_pattern::<UITextPattern>()
        .and_then(|pattern| pattern.get_document_range())
        .and_then(|range| range.get_text(LIMIT))
        .ok()
        .filter(|text| !text.is_empty())
        .or_else(|| {
            element
                .get_pattern::<UIValuePattern>()
                .and_then(|pattern| pattern.get_value())
                .ok()
        })
        .unwrap_or_else(|| element.get_name().unwrap_or_default());
    let note = if text.chars().count() >= LIMIT as usize {
        "\n… (cut at 20,000 characters)"
    } else {
        ""
    };
    format!("{}{note}", text.replace('\r', ""))
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
                // Line breaks stay visible as \n: squashed into spaces, a model retyping the
                // text joined the lines of a document.
                let lines = value
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .split('\n')
                    .map(clean)
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("value={}", quote(&clip(&lines, 80)))
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

/// What controlling an app can reach beyond the app itself, for approval warnings. Matched on the
/// process name, or on the name given to `launch`.
fn reach(app: &str) -> Option<&'static str> {
    let app = app
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(app)
        .trim_end_matches(".exe");
    if [
        "cmd",
        "powershell",
        "pwsh",
        "windowsterminal",
        "wt",
        "wsl",
        "bash",
        "openconsole",
        "conhost",
        "code",
        "cursor",
        "windsurf",
        "devenv",
        "idea64",
        "rider64",
    ]
    .contains(&app)
    {
        return Some("is equivalent to shell access");
    }
    if app == "explorer" {
        return Some("can read or write any file");
    }
    if [
        "systemsettings",
        "ms-settings:",
        "control",
        "regedit",
        "mmc",
        "taskmgr",
    ]
    .contains(&app)
    {
        return Some("can change system settings");
    }
    None
}

/// Whether a page document has nothing under it: Chromium has not built that page's tree yet.
fn cold_document(lines: &[String]) -> bool {
    let depth = |line: &String| line.len() - line.trim_start().len();
    lines.iter().enumerate().any(|(index, line)| {
        line.trim_start()
            .split_once("] ")
            .is_some_and(|(_, rest)| rest.starts_with("document "))
            && lines
                .get(index + 1)
                .is_none_or(|next| depth(next) <= depth(line))
    })
}

/// Whether a window shows UI Automation almost nothing beyond its title bar, as games, canvases,
/// video and remote desktops do. Those windows are read by OCR automatically, so the agent never
/// has to notice an empty tree and spend another call asking.
fn sparse(lines: &[String]) -> bool {
    lines
        .iter()
        .filter_map(|line| line.split('"').nth(1))
        .filter(|name| {
            !name.is_empty()
                && ![
                    "minimize",
                    "maximize",
                    "restore",
                    "close",
                    "system",
                    "system menu bar",
                ]
                .contains(&name.to_lowercase().as_str())
        })
        .count()
        < 3
}

/// A window's text by OCR, as lines to append to a snapshot or action result.
fn ocr(hwnd: isize) -> String {
    match capture(hwnd).and_then(|shot| shot.ocr()) {
        Ok(lines) if lines.is_empty() => {
            "\n  (little UI Automation text, and OCR found none)".into()
        }
        Ok(lines) => format!(
            "\n  read by OCR because the window shows little UI Automation text (click_at x,y):\n{}",
            lines
                .iter()
                .map(|line| format!("  {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        Err(error) => format!("\n  (little UI Automation text, and OCR failed: {error:#})"),
    }
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

#[cfg(test)]
mod tests {
    use super::{changes, clean, cold_document, quote, sparse};

    #[test]
    fn a_page_document_with_nothing_in_it_is_cold() {
        let lines = |items: &[&str]| {
            items
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
        };
        // Chrome after a reload: toolbar lines, then the page document alone.
        assert!(cold_document(&lines(&[
            r#"  [e1] button "Back""#,
            r#"  [e2] edit "Address and search bar" value="example.com""#,
            r#"  [e3] document "Example""#,
            r#"  [e4] button "Extensions""#,
        ])));
        assert!(!cold_document(&lines(&[
            r#"  [e3] document "Example""#,
            r#"    [e5] text "Example Domain""#,
        ])));
        assert!(!cold_document(&lines(&[r#"  [e1] button "Save""#])));
    }

    #[test]
    fn sparse_ignores_title_bar_controls() {
        let lines = |items: &[&str]| {
            items
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
        };
        assert!(sparse(&lines(&[
            r#"  [e1] titlebar """#,
            r#"    [e2] button "Minimize""#,
            r#"    [e3] button "Maximize""#,
            r#"    [e4] button "Close""#,
            r#"  [e5] pane "Game""#,
        ])));
        assert!(!sparse(&lines(&[
            r#"  [e1] text "Do you want to save changes?""#,
            r#"  [e2] button "Save""#,
            r#"  [e3] button "Don't save""#,
            r#"  [e4] button "Close""#,
        ])));
    }

    #[test]
    fn text_drops_invisible_marks_and_keeps_letters_readable() {
        assert_eq!(clean("\u{200e}1/\u{200e}6/\u{200e}2025"), "1/6/2025");
        assert_eq!(quote("café \"Ünïcode\""), r#""café \"Ünïcode\"""#);
        // A line break reads as \n, the way a model writes it back in JSON; a real backslash
        // followed by n stays distinguishable.
        assert_eq!(quote("Monday.\nBring"), r#""Monday.\nBring""#);
        assert_eq!(quote(r"C:\new"), r#""C:\\new""#);
    }

    #[test]
    fn changes_mark_changed_elements_and_count_long_removals() {
        let lines = |items: &[&str]| {
            items
                .iter()
                .map(|line| format!("  {line}"))
                .collect::<Vec<_>>()
        };
        let previous = lines(&[
            r#"[e1] button "Cryzo" checked"#,
            r#"[e2] tabitem "Month" selected"#,
            r#"[e3] text "No posts""#,
        ]);
        let now = lines(&[
            r#"[e1] button "Cryzo" unchecked"#,
            r#"[e2] tabitem "Month" selected"#,
            r#"[e4] button "Media post""#,
        ]);
        assert_eq!(
            changes("w", &previous, &now),
            "w changed:\n- [e3] text \"No posts\"\n~ [e1] button \"Cryzo\" unchecked\n+ [e4] button \"Media post\""
        );

        let grid = (0..20)
            .map(|day| format!("  [e{}] text \"{day}\"", day + 10))
            .collect::<Vec<_>>();
        assert_eq!(
            changes("w", &grid, &lines(&[r#"[e9] text "Done""#])),
            "w changed:\n- 20 elements gone\n+ [e9] text \"Done\""
        );
    }
}
