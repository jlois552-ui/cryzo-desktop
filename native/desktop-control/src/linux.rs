//! Desktop control on Linux: windows and elements through AT-SPI (the accessibility bus GNOME, KDE
//! and most toolkits speak), keys, clicks and screenshots through X11 with the XTEST extension. On
//! Wayland it works with apps running under XWayland; OCR uses `tesseract` when it is installed.

use std::{
    collections::HashMap,
    process::{Command, Stdio},
    sync::atomic::Ordering,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use atspi::{
    CoordType, Role, ScrollType, State,
    connection::AccessibilityConnection,
    proxy::{
        accessible::{AccessibleProxy, ObjectRefExt},
        proxy_ext::ProxyExt,
    },
};
use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{
            self, AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, ImageFormat,
            Keycode, Keysym,
        },
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
};

use crate::{
    Settings,
    common::{Rect, STOPPED},
    portable::{self, Backend, Info, Key, Modifier, Press, Window},
};

pub type Desktop = portable::Desktop<Linux>;
type Node = atspi::ObjectRefOwned;

impl portable::Desktop<Linux> {
    pub fn new(settings: Settings) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let bus = runtime.block_on(AccessibilityConnection::new()).context(
            "could not reach the accessibility bus (AT-SPI). Is a desktop session running?",
        )?;
        let x = X11::connect().ok();
        Ok(Self::with(Linux { runtime, bus, x }, settings))
    }
}

/// Watches for Control+Alt+Escape, the stop keys, by polling the X11 keyboard state.
pub fn start_stop_hotkey() {
    std::thread::spawn(|| {
        let Ok(x) = X11::connect() else {
            eprintln!("cryzo-desktop: no X11 display, so the Ctrl+Alt+Esc stop keys are off");
            return;
        };
        let codes = [0xffe3, 0xffe9, 0xff1b].map(|keysym| x.keycode(keysym).map(|(code, _)| code));
        let [Some(control), Some(alt), Some(escape)] = codes else {
            return;
        };
        loop {
            if let Ok(keys) = x.connection.query_keymap().map(|cookie| cookie.reply())
                && let Ok(keys) = keys
            {
                let down =
                    |code: Keycode| keys.keys[usize::from(code / 8)] & (1 << (code % 8)) != 0;
                if down(control) && down(alt) && down(escape) {
                    STOPPED.store(true, Ordering::SeqCst);
                    eprintln!("cryzo-desktop: stopped by Ctrl+Alt+Esc");
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
}

pub struct Linux {
    runtime: tokio::runtime::Runtime,
    bus: AccessibilityConnection,
    /// None without an X11 display (pure Wayland): reading still works, input doesn't.
    x: Option<X11>,
}

impl Linux {
    fn accessible(&self, node: &Node) -> Option<AccessibleProxy<'static>> {
        // Owned names make a proxy that outlives this call; zbus proxies hold their own connection.
        let name = node.name()?.clone();
        let path = node.path().clone();
        self.runtime.block_on(async {
            AccessibleProxy::builder(self.bus.connection())
                .destination(name)
                .ok()?
                .path(path)
                .ok()?
                .cache_properties(atspi::zbus::proxy::CacheProperties::No)
                .build()
                .await
                .ok()
        })
    }

    fn x(&self) -> Result<&X11> {
        self.x
            .as_ref()
            .ok_or_else(|| anyhow!("Linux computer use needs an X11 display (or XWayland) for keys, clicks and screenshots"))
    }

    fn extents(&self, node: &Node) -> Option<Rect> {
        let proxy = self.accessible(node)?;
        let (x, y, width, height) = self.runtime.block_on(async {
            proxy
                .proxies()
                .await
                .ok()?
                .component()
                .await
                .ok()?
                .get_extents(CoordType::Screen)
                .await
                .ok()
        })?;
        (width > 0 && height > 0).then_some(Rect {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        })
    }

    fn center(&self, node: &Node) -> Result<(i32, i32)> {
        let rect = self
            .extents(node)
            .ok_or_else(|| anyhow!("the element has no place on screen"))?;
        Ok(((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2))
    }

    /// Runs the element's first action whose name is one of `names`.
    fn action(&self, node: &Node, names: &[&str]) -> bool {
        let Some(proxy) = self.accessible(node) else {
            return false;
        };
        self.runtime.block_on(async {
            let Ok(proxies) = proxy.proxies().await else {
                return false;
            };
            let Ok(action) = proxies.action().await else {
                return false;
            };
            let Ok(actions) = action.get_actions().await else {
                return false;
            };
            let Some(index) = actions
                .iter()
                .position(|action| names.contains(&action.name.to_lowercase().as_str()))
            else {
                return false;
            };
            action.do_action(index as i32).await.unwrap_or(false)
        })
    }
}

impl Backend for Linux {
    type Node = Node;

    fn windows(&mut self) -> Result<Vec<Window<Node>>> {
        let connection = self.bus.connection().clone();
        let found = self.runtime.block_on(async move {
            let root = AccessibleProxy::builder(&connection)
                .destination("org.a11y.atspi.Registry")?
                .path("/org/a11y/atspi/accessible/root")?
                .build()
                .await?;
            let mut windows = Vec::new();
            for app in root.get_children().await.unwrap_or_default() {
                let Ok(app_proxy) = app.as_accessible_proxy(&connection).await else {
                    continue;
                };
                let app_name = app_proxy.name().await.unwrap_or_default();
                for window in app_proxy.get_children().await.unwrap_or_default() {
                    let Ok(proxy) = window.as_accessible_proxy(&connection).await else {
                        continue;
                    };
                    let role = proxy.get_role().await.unwrap_or(Role::Invalid);
                    if !matches!(
                        role,
                        Role::Frame | Role::Window | Role::Dialog | Role::Alert | Role::FileChooser
                    ) {
                        continue;
                    }
                    let states = proxy.get_state().await.unwrap_or_default();
                    if !states.contains(State::Showing) {
                        continue;
                    }
                    let title = proxy.name().await.unwrap_or_default();
                    let bounds = match proxy.proxies().await {
                        Ok(proxies) => match proxies.component().await {
                            Ok(component) => component.get_extents(CoordType::Screen).await.ok(),
                            Err(_) => None,
                        },
                        Err(_) => None,
                    };
                    windows.push((
                        window.clone(),
                        title,
                        app_name.clone(),
                        states.contains(State::Active),
                        bounds,
                    ));
                }
            }
            anyhow::Ok(windows)
        })?;
        let mut windows = found
            .into_iter()
            .map(|(node, title, app, focused, bounds)| Window {
                key: key(&node),
                title,
                app,
                focused,
                bounds: bounds.map(|(x, y, width, height)| Rect {
                    left: x,
                    top: y,
                    right: x + width,
                    bottom: y + height,
                }),
                node,
            })
            .collect::<Vec<_>>();
        // The active window first, as the foreground.
        windows.sort_by_key(|window| !window.focused);
        Ok(windows)
    }

    fn children(&mut self, node: &Node) -> Vec<Node> {
        let Some(proxy) = self.accessible(node) else {
            return Vec::new();
        };
        self.runtime
            .block_on(async { proxy.get_children().await })
            .unwrap_or_default()
    }

    fn info(&mut self, node: &Node) -> Option<Info> {
        let proxy = self.accessible(node)?;
        self.runtime.block_on(async {
            let atspi_role = proxy.get_role().await.ok()?;
            let states = proxy.get_state().await.unwrap_or_default();
            let mut name = proxy.name().await.unwrap_or_default();
            if name.trim().is_empty() {
                name = proxy.description().await.unwrap_or_default();
            }
            let role = role(atspi_role).to_string();
            let proxies = proxy.proxies().await.ok();
            let value = match role.as_str() {
                "edit" | "combobox" | "document" => match &proxies {
                    Some(proxies) => match proxies.text().await {
                        Ok(text) => text.get_text(0, 200).await.ok(),
                        Err(_) => None,
                    },
                    None => None,
                },
                "slider" | "spinner" | "progressbar" => match &proxies {
                    Some(proxies) => match proxies.value().await {
                        Ok(value) => value
                            .current_value()
                            .await
                            .ok()
                            .map(|value| value.to_string()),
                        Err(_) => None,
                    },
                    None => None,
                },
                _ => None,
            };
            let mut words = Vec::new();
            if !states.contains(State::Sensitive) && !states.contains(State::Enabled) {
                words.push("disabled".to_string());
            }
            if states.contains(State::Focused) {
                words.push("focused".into());
            }
            if matches!(role.as_str(), "checkbox" | "radiobutton" | "switch") {
                words.push(
                    if states.contains(State::Checked) {
                        "checked"
                    } else {
                        "unchecked"
                    }
                    .into(),
                );
            } else if states.contains(State::Checked) || states.contains(State::Pressed) {
                words.push("pressed".into());
            }
            if states.contains(State::Selected) {
                words.push("selected".into());
            }
            if states.contains(State::Expanded) {
                words.push("expanded".into());
            } else if states.contains(State::Expandable) {
                words.push("collapsed".into());
            }
            Some(Info {
                password: atspi_role == Role::PasswordText,
                hidden: !states.contains(State::Showing) && !states.contains(State::Visible),
                role,
                name,
                value,
                states: words,
            })
        })
    }

    fn identity(&mut self, node: &Node) -> Option<String> {
        Some(format!("{}{}", node.name_as_str()?, node.path_as_str()))
    }

    fn perform(&mut self, node: &Node, action: &str) -> Result<()> {
        match action {
            "click" | "toggle" => {
                if !self.action(
                    node,
                    &["click", "press", "activate", "toggle", "jump", "open"],
                ) {
                    let (x, y) = self.center(node)?;
                    self.x()?.click(x, y, "left")?;
                }
            }
            "double_click" | "right_click" => {
                let (x, y) = self.center(node)?;
                let button = if action == "right_click" {
                    "right"
                } else {
                    "double"
                };
                self.x()?.click(x, y, button)?;
            }
            "select" | "add_to_selection" => {
                if !self.action(node, &["select", "click", "activate", "press"]) {
                    let (x, y) = self.center(node)?;
                    self.x()?.click(x, y, "left")?;
                }
            }
            "expand" | "collapse" => {
                let names: &[&str] = if action == "expand" {
                    &["expand", "expand or contract", "toggle", "activate"]
                } else {
                    &["collapse", "expand or contract", "toggle", "activate"]
                };
                if !self.action(node, names) {
                    bail!("this element doesn't {action}");
                }
            }
            "scroll_into_view" => {
                let proxy = self.accessible(node).context("the element is gone")?;
                let scrolled = self.runtime.block_on(async {
                    proxy
                        .proxies()
                        .await
                        .ok()?
                        .component()
                        .await
                        .ok()?
                        .scroll_to(ScrollType::Anywhere)
                        .await
                        .ok()
                });
                if scrolled != Some(true) {
                    bail!("this element can't be scrolled into view");
                }
            }
            "scroll_down" | "scroll_up" => {
                let (x, y) = self.center(node)?;
                self.x()?.scroll(x, y, action == "scroll_down")?;
            }
            "focus" => {
                let proxy = self.accessible(node).context("the element is gone")?;
                let focused = self.runtime.block_on(async {
                    proxy
                        .proxies()
                        .await
                        .ok()?
                        .component()
                        .await
                        .ok()?
                        .grab_focus()
                        .await
                        .ok()
                });
                if focused != Some(true) {
                    bail!("this element can't take focus");
                }
            }
            _ => bail!("Unknown action {action:?}"),
        }
        Ok(())
    }

    fn set_text(&mut self, node: &Node, text: &str) -> Result<bool> {
        let Some(proxy) = self.accessible(node) else {
            return Ok(false);
        };
        Ok(self
            .runtime
            .block_on(async {
                proxy
                    .proxies()
                    .await
                    .ok()?
                    .editable_text()
                    .await
                    .ok()?
                    .set_text_contents(text)
                    .await
                    .ok()
            })
            .unwrap_or(false))
    }

    fn text(&mut self, node: &Node) -> String {
        let Some(proxy) = self.accessible(node) else {
            return String::new();
        };
        self.runtime.block_on(async {
            match proxy.proxies().await {
                Ok(proxies) => match proxies.text().await {
                    Ok(text) => text.get_text(0, -1).await.unwrap_or_default(),
                    Err(_) => proxy.name().await.unwrap_or_default(),
                },
                Err(_) => proxy.name().await.unwrap_or_default(),
            }
        })
    }

    fn raise(&mut self, window: &Window<Node>) -> Result<()> {
        self.x()?.activate(&window.title)
    }

    fn press(&mut self, press: &Press) -> Result<()> {
        let x = self.x()?;
        let keysym = match &press.key {
            Key::Named(name) => named(name).context("unknown key")?,
            Key::Char(char) => keysym(*char),
        };
        let modifiers = press
            .modifiers
            .iter()
            .map(|modifier| match modifier {
                Modifier::Ctrl => 0xffe3,
                Modifier::Alt => 0xffe9,
                Modifier::Shift => 0xffe1,
                Modifier::Meta => 0xffeb,
            })
            .collect::<Vec<_>>();
        x.press(&modifiers, keysym)
    }

    fn type_text(&mut self, text: &str) -> Result<()> {
        let x = self.x()?;
        for char in text.chars() {
            crate::common::stopped()?;
            match char {
                '\n' => x.press(&[], 0xff0d)?,
                '\t' => x.press(&[], 0xff09)?,
                char => x.press(&[], keysym(char))?,
            }
        }
        Ok(())
    }

    fn click(&mut self, x: i32, y: i32, button: &str) -> Result<()> {
        self.x()?.click(x, y, button)
    }

    fn launch(&mut self, app: &str) -> Result<()> {
        let target = app.trim();
        // URLs and files open with their default app; otherwise a desktop entry or a program.
        let launched =
            if target.contains("://") || target.starts_with('/') || target.starts_with('~') {
                Command::new("xdg-open")
                    .arg(target)
                    .stdin(Stdio::null())
                    .spawn()
                    .is_ok()
            } else {
                Command::new("gtk-launch")
                    .arg(target)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
                    || Command::new(target).stdin(Stdio::null()).spawn().is_ok()
            };
        if !launched {
            bail!("could not open {target:?}");
        }
        Ok(())
    }

    fn screenshot(&mut self, window: &Window<Node>) -> Result<Vec<u8>> {
        let bounds = window.bounds.context("the window has no place on screen")?;
        self.x()?.jpeg(bounds)
    }

    fn ocr(&mut self, window: &Window<Node>) -> Result<Vec<String>> {
        let bounds = window.bounds.context("the window has no place on screen")?;
        let jpeg = self.x()?.jpeg(bounds)?;
        let path = std::env::temp_dir().join(format!("cryzo-ocr-{}.jpg", window.key));
        std::fs::write(&path, jpeg)?;
        let output = Command::new("tesseract")
            .arg(&path)
            .args(["-", "tsv"])
            .stderr(Stdio::null())
            .output();
        let _ = std::fs::remove_file(&path);
        let output = output.map_err(|_| anyhow!("install tesseract-ocr to read windows by OCR"))?;
        Ok(ocr_lines(&String::from_utf8_lossy(&output.stdout), bounds))
    }

    fn bounds(&mut self, node: &Node) -> Option<Rect> {
        self.extents(node)
    }

    fn start_overlay(&mut self) -> Option<crate::motion::Indicator> {
        crate::linux_overlay::start()
    }

    fn app_at(&mut self, _x: i32, _y: i32) -> Option<String> {
        None
    }
}

/// A short stable key for a window, from its bus name and path.
fn key(node: &Node) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    node.name_as_str().hash(&mut hasher);
    node.path_as_str().hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}

/// `tesseract … tsv` output as lines of text ending with their centre on screen.
fn ocr_lines(tsv: &str, bounds: Rect) -> Vec<String> {
    let mut lines: Vec<(String, (i32, i32, i32, i32))> = Vec::new();
    let mut index = HashMap::new();
    for row in tsv.lines().skip(1) {
        let fields = row.split('\t').collect::<Vec<_>>();
        if fields.len() < 12 || fields[0] != "5" {
            continue;
        }
        let text = fields[11].trim();
        let number = |at: usize| fields[at].parse::<i32>().unwrap_or(0);
        if text.is_empty() {
            continue;
        }
        let line = (fields[2], fields[3], fields[4]);
        let (left, top, width, height) = (number(6), number(7), number(8), number(9));
        let entry = *index.entry(line).or_insert_with(|| {
            lines.push((String::new(), (left, top, left + width, top + height)));
            lines.len() - 1
        });
        let (words, (x0, y0, x1, y1)) = &mut lines[entry];
        if !words.is_empty() {
            words.push(' ');
        }
        words.push_str(text);
        *x0 = (*x0).min(left);
        *y0 = (*y0).min(top);
        *x1 = (*x1).max(left + width);
        *y1 = (*y1).max(top + height);
    }
    lines
        .into_iter()
        .map(|(text, (x0, y0, x1, y1))| {
            format!(
                "{text} @{},{}",
                bounds.left + (x0 + x1) / 2,
                bounds.top + (y0 + y1) / 2
            )
        })
        .collect()
}

/// The Windows-style role name for an AT-SPI role.
fn role(role: Role) -> &'static str {
    match role {
        Role::Button | Role::PushButtonMenu => "button",
        Role::ToggleButton => "button",
        Role::Entry | Role::Text | Role::PasswordText | Role::Editbar | Role::Autocomplete => {
            "edit"
        }
        Role::ComboBox => "combobox",
        Role::Label | Role::Static | Role::Paragraph | Role::Caption => "text",
        Role::Link => "hyperlink",
        Role::CheckBox | Role::CheckMenuItem => "checkbox",
        Role::RadioButton | Role::RadioMenuItem => "radiobutton",
        Role::MenuItem | Role::TearoffMenuItem => "menuitem",
        Role::Menu | Role::MenuBar | Role::PopupMenu => "menu",
        Role::PageTab => "tabitem",
        Role::PageTabList => "tab",
        Role::ListItem | Role::TableCell | Role::TableRow => "listitem",
        Role::TreeItem => "treeitem",
        Role::List | Role::ListBox | Role::Table | Role::Tree | Role::TreeTable => "list",
        Role::Image | Role::Icon => "image",
        Role::Slider => "slider",
        Role::SpinButton => "spinner",
        Role::ProgressBar | Role::LevelBar => "progressbar",
        Role::DocumentFrame | Role::DocumentWeb | Role::DocumentText | Role::Terminal => "document",
        Role::ScrollPane | Role::Viewport => "pane",
        Role::ToolBar => "toolbar",
        Role::Frame | Role::Window | Role::Dialog | Role::Alert | Role::FileChooser => "window",
        Role::Heading => "heading",
        Role::Separator => "separator",
        Role::StatusBar => "statusbar",
        _ => "group",
    }
}

/// X11 keysyms for named keys.
fn named(name: &str) -> Option<Keysym> {
    Some(match name {
        "enter" => 0xff0d,
        "tab" => 0xff09,
        "escape" => 0xff1b,
        "backspace" => 0xff08,
        "delete" => 0xffff,
        "space" => 0x20,
        "up" => 0xff52,
        "down" => 0xff54,
        "left" => 0xff51,
        "right" => 0xff53,
        "home" => 0xff50,
        "end" => 0xff57,
        "pageup" => 0xff55,
        "pagedown" => 0xff56,
        "insert" => 0xff63,
        name if name.starts_with('f') => 0xffbe + name[1..].parse::<u32>().ok()?.checked_sub(1)?,
        _ => return None,
    })
}

/// The keysym for a character: Latin-1 directly, anything else as a Unicode keysym.
fn keysym(char: char) -> Keysym {
    let code = u32::from(char);
    if (0x20..=0x7e).contains(&code) || (0xa0..=0xff).contains(&code) {
        code
    } else {
        0x0100_0000 | code
    }
}

/// The X11 connection for input and screenshots.
struct X11 {
    connection: RustConnection,
    root: xproto::Window,
    min_keycode: Keycode,
    max_keycode: Keycode,
}

impl X11 {
    fn connect() -> Result<Self> {
        let (connection, screen) = x11rb::connect(None).context("no X11 display")?;
        let setup = connection.setup();
        let root = setup.roots[screen].root;
        let (min_keycode, max_keycode) = (setup.min_keycode, setup.max_keycode);
        Ok(Self {
            connection,
            root,
            min_keycode,
            max_keycode,
        })
    }

    /// The keycode that types `keysym`, and whether it needs Shift.
    fn keycode(&self, keysym: Keysym) -> Option<(Keycode, bool)> {
        let count = self.max_keycode - self.min_keycode + 1;
        let mapping = self
            .connection
            .get_keyboard_mapping(self.min_keycode, count)
            .ok()?
            .reply()
            .ok()?;
        let per = usize::from(mapping.keysyms_per_keycode);
        let index = mapping.keysyms.iter().position(|found| *found == keysym)?;
        Some((self.min_keycode + (index / per) as u8, index % per == 1))
    }

    /// Presses a key with modifiers held. A character the keyboard map lacks is typed through a
    /// spare keycode remapped for the moment, as xdotool does.
    fn press(&self, modifiers: &[Keysym], keysym: Keysym) -> Result<()> {
        let (code, shift, remapped) = match self.keycode(keysym) {
            Some((code, shift)) => (code, shift, None),
            None => {
                let spare = self
                    .spare()
                    .context("no spare key to type this character")?;
                self.connection
                    .change_keyboard_mapping(1, spare, 1, &[keysym])?
                    .check()?;
                std::thread::sleep(Duration::from_millis(20));
                (spare, false, Some(spare))
            }
        };
        let mut held = modifiers
            .iter()
            .filter_map(|modifier| self.keycode(*modifier).map(|(code, _)| code))
            .collect::<Vec<_>>();
        if shift && let Some((code, _)) = self.keycode(0xffe1) {
            held.push(code);
        }
        for code in &held {
            self.fake(xproto::KEY_PRESS_EVENT, *code)?;
        }
        self.fake(xproto::KEY_PRESS_EVENT, code)?;
        self.fake(xproto::KEY_RELEASE_EVENT, code)?;
        for code in held.iter().rev() {
            self.fake(xproto::KEY_RELEASE_EVENT, *code)?;
        }
        self.connection.flush()?;
        std::thread::sleep(Duration::from_millis(8));
        if let Some(spare) = remapped {
            self.connection
                .change_keyboard_mapping(1, spare, 1, &[0])?
                .check()?;
        }
        Ok(())
    }

    /// A keycode with nothing mapped to it.
    fn spare(&self) -> Option<Keycode> {
        let count = self.max_keycode - self.min_keycode + 1;
        let mapping = self
            .connection
            .get_keyboard_mapping(self.min_keycode, count)
            .ok()?
            .reply()
            .ok()?;
        let per = usize::from(mapping.keysyms_per_keycode);
        mapping
            .keysyms
            .chunks(per)
            .position(|keysyms| keysyms.iter().all(|keysym| *keysym == 0))
            .map(|index| self.min_keycode + index as u8)
    }

    fn fake(&self, kind: u8, detail: u8) -> Result<()> {
        self.connection
            .xtest_fake_input(kind, detail, 0, self.root, 0, 0, 0)?;
        Ok(())
    }

    fn click(&self, x: i32, y: i32, button: &str) -> Result<()> {
        self.connection.xtest_fake_input(
            xproto::MOTION_NOTIFY_EVENT,
            0,
            0,
            self.root,
            x as i16,
            y as i16,
            0,
        )?;
        let (number, times) = match button {
            "right" => (3, 1),
            "double" => (1, 2),
            _ => (1, 1),
        };
        for _ in 0..times {
            self.fake(xproto::BUTTON_PRESS_EVENT, number)?;
            self.fake(xproto::BUTTON_RELEASE_EVENT, number)?;
        }
        self.connection.flush()?;
        Ok(())
    }

    fn scroll(&self, x: i32, y: i32, down: bool) -> Result<()> {
        self.connection.xtest_fake_input(
            xproto::MOTION_NOTIFY_EVENT,
            0,
            0,
            self.root,
            x as i16,
            y as i16,
            0,
        )?;
        let button = if down { 5 } else { 4 };
        for _ in 0..5 {
            self.fake(xproto::BUTTON_PRESS_EVENT, button)?;
            self.fake(xproto::BUTTON_RELEASE_EVENT, button)?;
        }
        self.connection.flush()?;
        Ok(())
    }

    /// Activates the top-level window titled `title` through the window manager (EWMH).
    fn activate(&self, title: &str) -> Result<()> {
        let atom = |name: &[u8]| -> Result<u32> {
            Ok(self.connection.intern_atom(false, name)?.reply()?.atom)
        };
        let (list, active, name, utf8) = (
            atom(b"_NET_CLIENT_LIST")?,
            atom(b"_NET_ACTIVE_WINDOW")?,
            atom(b"_NET_WM_NAME")?,
            atom(b"UTF8_STRING")?,
        );
        let clients = self
            .connection
            .get_property(false, self.root, list, AtomEnum::WINDOW, 0, 4096)?
            .reply()?;
        let window = clients
            .value32()
            .into_iter()
            .flatten()
            .find(|window| {
                self.connection
                    .get_property(false, *window, name, utf8, 0, 1024)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .is_some_and(|reply| String::from_utf8_lossy(&reply.value) == title)
            })
            .context("the window manager doesn't list this window")?;
        let event = ClientMessageEvent::new(32, window, active, [2, 0, 0, 0, 0]);
        self.connection.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )?;
        self.connection.flush()?;
        std::thread::sleep(Duration::from_millis(150));
        Ok(())
    }

    /// The screen area as JPEG.
    fn jpeg(&self, area: Rect) -> Result<Vec<u8>> {
        let (width, height) = (
            u16::try_from(area.right - area.left)?,
            u16::try_from(area.bottom - area.top)?,
        );
        let image = self
            .connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.root,
                area.left as i16,
                area.top as i16,
                width,
                height,
                !0,
            )?
            .reply()?;
        // 24- and 32-bit visuals store pixels as BGRX.
        let pixels = usize::from(width) * usize::from(height);
        if image.data.len() < pixels * 4 {
            bail!("the display's pixel format isn't supported for screenshots");
        }
        let rgb = image
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .take(pixels)
            .flat_map(|pixel| [pixel[2], pixel[1], pixel[0]])
            .collect::<Vec<_>>();
        let mut jpeg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg, 70).encode(
            &rgb,
            width,
            height,
            jpeg_encoder::ColorType::Rgb,
        )?;
        Ok(jpeg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characters_and_keys_map_to_keysyms() {
        assert_eq!(keysym('a'), 0x61);
        assert_eq!(keysym('é'), 0xe9);
        assert_eq!(keysym('€'), 0x0100_20ac);
        assert_eq!(named("f5"), Some(0xffc2));
        assert_eq!(named("enter"), Some(0xff0d));
    }

    #[test]
    fn tesseract_words_become_lines_with_points() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t10\t20\t30\t10\t90\tSave\n\
5\t1\t1\t1\t1\t2\t50\t20\t30\t10\t90\tfile\n";
        let bounds = Rect {
            left: 100,
            top: 200,
            right: 600,
            bottom: 500,
        };
        assert_eq!(
            ocr_lines(tsv, bounds),
            vec!["Save file @145,225".to_string()]
        );
    }
}
