//! Desktop control on macOS: windows and elements through the Accessibility API (AXUIElement),
//! keys and the mouse through CoreGraphics events, screenshots through `screencapture`. Needs the
//! Accessibility permission, and Screen Recording for screenshots.

use std::{
    ffi::c_void,
    process::Command,
    time::{Duration, Instant},
};

use accessibility_sys::{
    AXError, AXIsProcessTrusted, AXIsProcessTrustedWithOptions, AXUIElementCopyAttributeValue,
    AXUIElementCopyElementAtPosition, AXUIElementCreateApplication, AXUIElementCreateSystemWide,
    AXUIElementGetPid, AXUIElementIsAttributeSettable, AXUIElementPerformAction, AXUIElementRef,
    AXUIElementSetAttributeValue, AXValueGetType, AXValueGetValue, AXValueRef, kAXErrorSuccess,
    kAXTrustedCheckOptionPrompt, kAXValueTypeCGPoint, kAXValueTypeCGSize,
};
use anyhow::{Context, Result, anyhow, bail};
use core_foundation::{
    array::CFArray,
    base::{CFHash, CFRelease, CFRetain, CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use core_graphics::{
    event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField},
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::{CGPoint, CGSize},
    window::{
        copy_window_info, kCGNullWindowID, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowNumber, kCGWindowOwnerName, kCGWindowOwnerPID,
    },
};

use crate::{
    Settings,
    common::{Rect, STOPPED},
    portable::{self, Backend, Info, Key, Modifier, Press, Window},
};

pub type Desktop = portable::Desktop<Mac>;

impl portable::Desktop<Mac> {
    pub fn new(settings: Settings) -> Result<Self> {
        // SAFETY: plain calls into ApplicationServices with a dictionary we own.
        let trusted = unsafe { AXIsProcessTrusted() };
        if !trusted {
            let options = CFDictionary::from_CFType_pairs(&[(
                unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) },
                CFBoolean::true_value().as_CFType(),
            )]);
            unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) };
            bail!(
                "Computer use needs the Accessibility permission. In System Settings > Privacy & Security > Accessibility, turn on Cryzo (or the app running cryzo-desktop-mcp), then try again."
            );
        }
        Ok(Self::with(Mac, settings))
    }
}

/// Watches for Control+Option+Escape, the stop keys, by polling the keyboard state. Polling needs
/// no event tap or run loop, and the Accessibility permission covers it.
pub fn start_stop_hotkey() {
    std::thread::spawn(|| {
        loop {
            // SAFETY: a read-only query of the combined keyboard state.
            let down = |key: u16| unsafe { CGEventSourceKeyState(COMBINED_SESSION_STATE, key) };
            if (down(0x3B) || down(0x3E)) && (down(0x3A) || down(0x3D)) && down(0x35) {
                STOPPED.store(true, std::sync::atomic::Ordering::SeqCst);
                eprintln!("cryzo-desktop: stopped by Control+Option+Escape");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
}

const COMBINED_SESSION_STATE: i32 = 0;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    /// The window number behind an AX window, the same one `CGWindowList` and `screencapture`
    /// use. Private but long stable; window managers such as yabai and Rectangle rely on it.
    fn _AXUIElementGetWindow(element: AXUIElementRef, window: *mut u32) -> AXError;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
}

pub struct Mac;

/// A retained AXUIElement.
pub struct Element(AXUIElementRef);

impl Clone for Element {
    fn clone(&self) -> Self {
        // SAFETY: the element is live; the clone holds its own retain.
        unsafe { CFRetain(self.0 as CFTypeRef) };
        Self(self.0)
    }
}

impl Drop for Element {
    fn drop(&mut self) {
        // SAFETY: each Element holds exactly one retain.
        unsafe { CFRelease(self.0 as CFTypeRef) };
    }
}

impl Element {
    /// Takes ownership of a reference from a Create or Copy call.
    fn owned(element: AXUIElementRef) -> Option<Self> {
        (!element.is_null()).then_some(Self(element))
    }

    fn application(pid: i32) -> Option<Self> {
        // SAFETY: returns a new reference, or null.
        Self::owned(unsafe { AXUIElementCreateApplication(pid) })
    }

    fn attribute(&self, name: &str) -> Option<CFType> {
        let name = CFString::new(name);
        let mut value: CFTypeRef = std::ptr::null();
        // SAFETY: the element and the name are live; on success we own `value`.
        let error = unsafe {
            AXUIElementCopyAttributeValue(self.0, name.as_concrete_TypeRef(), &mut value)
        };
        (error == kAXErrorSuccess && !value.is_null())
            .then(|| unsafe { CFType::wrap_under_create_rule(value) })
    }

    fn string(&self, name: &str) -> Option<String> {
        self.attribute(name)?
            .downcast::<CFString>()
            .map(|text| text.to_string())
    }

    fn bool(&self, name: &str) -> Option<bool> {
        self.attribute(name)?
            .downcast::<CFBoolean>()
            .map(bool::from)
    }

    fn elements(&self, name: &str) -> Vec<Element> {
        let Some(array) = self
            .attribute(name)
            .and_then(|value| value.downcast::<CFArray>())
        else {
            return Vec::new();
        };
        array
            .iter()
            .filter_map(|item| {
                let element = *item as AXUIElementRef;
                // SAFETY: the array holds live elements; each Element takes its own retain.
                (!element.is_null()).then(|| {
                    unsafe { CFRetain(element as CFTypeRef) };
                    Element(element)
                })
            })
            .collect()
    }

    /// A point or size attribute.
    fn geometry<T: Default>(&self, name: &str, kind: u32) -> Option<T> {
        let value = self.attribute(name)?;
        let value = value.as_CFTypeRef() as AXValueRef;
        let mut out = T::default();
        // SAFETY: we check the AXValue's type before reading it into a T of that type.
        let read = unsafe {
            AXValueGetType(value) == kind
                && AXValueGetValue(value, kind, (&raw mut out).cast::<c_void>())
        };
        read.then_some(out)
    }

    fn frame(&self) -> Option<Rect> {
        let position: CGPoint = self.geometry("AXPosition", kAXValueTypeCGPoint)?;
        let size: CGSize = self.geometry("AXSize", kAXValueTypeCGSize)?;
        Some(Rect {
            left: position.x.round() as i32,
            top: position.y.round() as i32,
            right: (position.x + size.width).round() as i32,
            bottom: (position.y + size.height).round() as i32,
        })
    }

    fn settable(&self, name: &str) -> bool {
        let name = CFString::new(name);
        let mut settable = 0u8;
        // SAFETY: live element and name; writes one byte.
        let error = unsafe {
            AXUIElementIsAttributeSettable(
                self.0,
                name.as_concrete_TypeRef(),
                (&raw mut settable).cast(),
            )
        };
        error == kAXErrorSuccess && settable != 0
    }

    fn set(&self, name: &str, value: &CFType) -> bool {
        let name = CFString::new(name);
        // SAFETY: live element, name and value.
        unsafe {
            AXUIElementSetAttributeValue(self.0, name.as_concrete_TypeRef(), value.as_CFTypeRef())
                == kAXErrorSuccess
        }
    }

    fn act(&self, action: &str) -> bool {
        let action = CFString::new(action);
        // SAFETY: live element and action name.
        unsafe { AXUIElementPerformAction(self.0, action.as_concrete_TypeRef()) == kAXErrorSuccess }
    }

    fn pid(&self) -> Option<i32> {
        let mut pid = 0;
        // SAFETY: live element; writes the pid.
        (unsafe { AXUIElementGetPid(self.0, &mut pid) } == kAXErrorSuccess).then_some(pid)
    }

    fn window_number(&self) -> Option<u32> {
        let mut number = 0u32;
        // SAFETY: live element; writes the window number.
        (unsafe { _AXUIElementGetWindow(self.0, &mut number) } == kAXErrorSuccess && number != 0)
            .then_some(number)
    }
}

/// On-screen app windows, front to back, from the window server: (window number, pid, app).
fn window_list() -> Vec<(u32, i32, String)> {
    let Some(list) = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            // SAFETY: CGWindowListCopyWindowInfo returns an array of dictionaries.
            let entry: CFDictionary<CFString, CFType> =
                unsafe { CFDictionary::wrap_under_get_rule(*item as _) };
            let number = |key| {
                entry
                    .find(unsafe { CFString::wrap_under_get_rule(key) })
                    .and_then(|value| value.downcast::<CFNumber>())
                    .and_then(|value| value.to_i64())
            };
            // Layer 0 holds normal app windows; menus, the Dock and overlays sit above it.
            (number(unsafe { kCGWindowLayer }) == Some(0)).then_some(())?;
            let app = entry
                .find(unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerName) })
                .and_then(|value| value.downcast::<CFString>())
                .map(|name| name.to_string())
                .unwrap_or_default();
            Some((
                number(unsafe { kCGWindowNumber })? as u32,
                number(unsafe { kCGWindowOwnerPID })? as i32,
                app,
            ))
        })
        .collect()
}

impl Backend for Mac {
    type Node = Element;

    fn windows(&mut self) -> Result<Vec<Window<Element>>> {
        let list = window_list();
        let mut found = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (_, pid, app) in &list {
            if !seen.insert(*pid) {
                continue;
            }
            let Some(application) = Element::application(*pid) else {
                continue;
            };
            for window in application.elements("AXWindows") {
                let Some(number) = window.window_number() else {
                    continue;
                };
                found.push((number, app.clone(), window));
            }
        }
        // Front to back as the window server orders them; the first is the focused window.
        let mut windows = list
            .iter()
            .filter_map(|(number, _, _)| {
                let index = found.iter().position(|(found, _, _)| found == number)?;
                let (number, app, window) = found.swap_remove(index);
                Some(Window {
                    key: format!("{number:x}"),
                    title: window.string("AXTitle").unwrap_or_default(),
                    app,
                    focused: false,
                    bounds: window.frame(),
                    node: window,
                })
            })
            .collect::<Vec<_>>();
        if let Some(first) = windows.first_mut() {
            first.focused = true;
        }
        Ok(windows)
    }

    fn children(&mut self, node: &Element) -> Vec<Element> {
        node.elements("AXChildren")
    }

    fn info(&mut self, node: &Element) -> Option<Info> {
        let ax_role = node.string("AXRole")?;
        let subrole = node.string("AXSubrole").unwrap_or_default();
        let role = role(&ax_role, &subrole).to_string();
        let name = [
            node.string("AXTitle"),
            node.string("AXDescription"),
            node.string("AXHelp"),
        ]
        .into_iter()
        .flatten()
        .find(|text| !text.trim().is_empty())
        .unwrap_or_default();
        let raw = node.attribute("AXValue");
        let mut states = Vec::new();
        let value = match role.as_str() {
            "checkbox" | "radiobutton" | "switch" => {
                let on = raw
                    .as_ref()
                    .and_then(|value| value.downcast::<CFNumber>())
                    .and_then(|value| value.to_i64());
                match on {
                    Some(1) => states.push("checked".into()),
                    Some(0) => states.push("unchecked".into()),
                    Some(_) => states.push("mixed".into()),
                    None => {}
                }
                None
            }
            // A static text's value is its text, which is its name here.
            "text" => None,
            _ => raw.and_then(|value| {
                value
                    .downcast::<CFString>()
                    .map(|text| text.to_string())
                    .or_else(|| {
                        value
                            .downcast::<CFNumber>()
                            .and_then(|number| number.to_f64())
                            .map(|number| number.to_string())
                    })
            }),
        };
        let name = if name.is_empty() && role == "text" {
            node.string("AXValue").unwrap_or_default()
        } else {
            name
        };
        if node.bool("AXEnabled") == Some(false) {
            states.push("disabled".into());
        }
        if node.bool("AXFocused") == Some(true) {
            states.push("focused".into());
        }
        if node.bool("AXSelected") == Some(true) {
            states.push("selected".into());
        }
        match node.bool("AXExpanded") {
            Some(true) => states.push("expanded".into()),
            Some(false) if role == "treeitem" || role == "combobox" => {
                states.push("collapsed".into())
            }
            _ => {}
        }
        Some(Info {
            role,
            name,
            value,
            states,
            password: subrole == "AXSecureTextField",
            hidden: node.bool("AXHidden") == Some(true),
        })
    }

    fn identity(&mut self, node: &Element) -> Option<String> {
        // SAFETY: CFHash on a live element; equal elements hash equally.
        let hash = unsafe { CFHash(node.0 as CFTypeRef) };
        Some(format!("{}:{hash:x}", node.pid()?))
    }

    fn perform(&mut self, node: &Element, action: &str) -> Result<()> {
        let center = || {
            node.frame()
                .map(|rect| ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2))
                .ok_or_else(|| anyhow!("the element has no place on screen"))
        };
        match action {
            "click" | "toggle" => {
                if !node.act("AXPress") {
                    let (x, y) = center()?;
                    click(x, y, "left")?;
                }
            }
            "double_click" => {
                let (x, y) = center()?;
                click(x, y, "double")?;
            }
            "right_click" => {
                if !node.act("AXShowMenu") {
                    let (x, y) = center()?;
                    click(x, y, "right")?;
                }
            }
            "select" | "add_to_selection" => {
                let selected = node.settable("AXSelected")
                    && node.set("AXSelected", &CFBoolean::true_value().as_CFType());
                if !selected && !node.act("AXPress") {
                    let (x, y) = center()?;
                    click(x, y, "left")?;
                }
            }
            "expand" | "collapse" => {
                let open = action == "expand";
                let value = if open {
                    CFBoolean::true_value()
                } else {
                    CFBoolean::false_value()
                };
                let set = node.settable("AXExpanded") && node.set("AXExpanded", &value.as_CFType());
                if !set && node.bool("AXExpanded") != Some(open) && !node.act("AXPress") {
                    bail!("this element doesn't {action}");
                }
            }
            "scroll_into_view" => {
                if !node.act("AXScrollToVisible") {
                    bail!("this element can't be scrolled into view");
                }
            }
            "scroll_down" | "scroll_up" => {
                let (x, y) = center()?;
                scroll(x, y, if action == "scroll_down" { -8 } else { 8 })?;
            }
            "focus" => {
                if !node.set("AXFocused", &CFBoolean::true_value().as_CFType()) {
                    bail!("this element can't take focus");
                }
            }
            _ => bail!("Unknown action {action:?}"),
        }
        Ok(())
    }

    fn set_text(&mut self, node: &Element, text: &str) -> Result<bool> {
        Ok(node.settable("AXValue") && node.set("AXValue", &CFString::new(text).as_CFType()))
    }

    fn text(&mut self, node: &Element) -> String {
        node.string("AXValue")
            .or_else(|| node.string("AXTitle"))
            .or_else(|| node.string("AXDescription"))
            .unwrap_or_default()
    }

    fn raise(&mut self, window: &Window<Element>) -> Result<()> {
        if let Some(application) = window.node.pid().and_then(Element::application) {
            application.set("AXFrontmost", &CFBoolean::true_value().as_CFType());
        }
        window.node.act("AXRaise");
        window
            .node
            .set("AXMain", &CFBoolean::true_value().as_CFType());
        std::thread::sleep(Duration::from_millis(150));
        Ok(())
    }

    fn press(&mut self, press: &Press) -> Result<()> {
        let (code, shift) = match &press.key {
            Key::Named(name) => (named(name).context("unknown key")?, false),
            Key::Char(char) => match character(char.to_ascii_lowercase()) {
                Some(code) => (code, char.is_ascii_uppercase()),
                // Anything else types as text.
                None if press.modifiers.is_empty() => return type_text(&char.to_string()),
                None => bail!("can't press {char:?} with modifiers on this keyboard layout"),
            },
        };
        let mut flags = CGEventFlags::empty();
        for modifier in &press.modifiers {
            flags |= match modifier {
                Modifier::Ctrl => CGEventFlags::CGEventFlagControl,
                Modifier::Alt => CGEventFlags::CGEventFlagAlternate,
                Modifier::Shift => CGEventFlags::CGEventFlagShift,
                Modifier::Meta => CGEventFlags::CGEventFlagCommand,
            };
        }
        if shift {
            flags |= CGEventFlags::CGEventFlagShift;
        }
        for down in [true, false] {
            let event = CGEvent::new_keyboard_event(source()?, code, down)
                .map_err(|()| anyhow!("could not create a key event"))?;
            event.set_flags(flags);
            event.post(CGEventTapLocation::HID);
        }
        std::thread::sleep(Duration::from_millis(8));
        Ok(())
    }

    fn type_text(&mut self, text: &str) -> Result<()> {
        type_text(text)
    }

    fn click(&mut self, x: i32, y: i32, button: &str) -> Result<()> {
        click(x, y, button)
    }

    fn launch(&mut self, app: &str) -> Result<()> {
        let target = app.trim();
        // A URL, a path or a bundle opens as itself; a bare name is an app.
        let mut command = Command::new("open");
        if target.contains("://") || target.starts_with('/') || target.starts_with('~') {
            command.arg(target);
        } else {
            command.args(["-a", target]);
        }
        let status = command.status().context("could not run open")?;
        if !status.success() {
            bail!("macOS could not open {target:?}");
        }
        Ok(())
    }

    fn screenshot(&mut self, window: &Window<Element>) -> Result<Vec<u8>> {
        let path = std::env::temp_dir().join(format!("cryzo-shot-{}.jpg", window.key));
        let number = u32::from_str_radix(&window.key, 16)?;
        let status = Command::new("screencapture")
            .args(["-x", "-o", "-t", "jpg", &format!("-l{number}")])
            .arg(&path)
            .status()
            .context("could not run screencapture")?;
        let shot = std::fs::read(&path);
        let _ = std::fs::remove_file(&path);
        if !status.success() {
            bail!(
                "macOS refused the screenshot. Allow Cryzo in System Settings > Privacy & Security > Screen Recording."
            );
        }
        Ok(shot?)
    }

    fn ocr(&mut self, _window: &Window<Element>) -> Result<Vec<String>> {
        bail!("OCR isn't available on macOS yet; use screenshot for visual content")
    }

    fn app_at(&mut self, x: i32, y: i32) -> Option<String> {
        // SAFETY: a new system-wide element, released by Element; the hit element is ours too.
        let system = Element::owned(unsafe { AXUIElementCreateSystemWide() })?;
        let mut hit: AXUIElementRef = std::ptr::null_mut();
        let error =
            unsafe { AXUIElementCopyElementAtPosition(system.0, x as f32, y as f32, &mut hit) };
        let hit = (error == kAXErrorSuccess)
            .then(|| Element::owned(hit))
            .flatten()?;
        let pid = hit.pid()?;
        window_list()
            .into_iter()
            .find(|(_, owner, _)| *owner == pid)
            .map(|(_, _, app)| app)
    }
}

/// The Windows-style role name for an AX role.
fn role(role: &str, subrole: &str) -> &'static str {
    match (role, subrole) {
        ("AXRadioButton", "AXTabButton") => "tabitem",
        ("AXCheckBox", "AXSwitch") => "switch",
        ("AXButton", _) | ("AXMenuButton", _) | ("AXDisclosureTriangle", _) => "button",
        ("AXTextField", _) | ("AXTextArea", _) | ("AXSearchField", _) => "edit",
        ("AXComboBox", _) | ("AXPopUpButton", _) => "combobox",
        ("AXStaticText", _) => "text",
        ("AXLink", _) => "hyperlink",
        ("AXCheckBox", _) => "checkbox",
        ("AXRadioButton", _) => "radiobutton",
        ("AXMenuItem", _) | ("AXMenuBarItem", _) => "menuitem",
        ("AXMenu", _) | ("AXMenuBar", _) => "menu",
        ("AXTabGroup", _) => "tab",
        ("AXRow", "AXOutlineRow") => "treeitem",
        ("AXRow", _) | ("AXCell", _) => "listitem",
        ("AXList", _) | ("AXTable", _) | ("AXOutline", _) | ("AXBrowser", _) => "list",
        ("AXImage", _) => "image",
        ("AXSlider", _) => "slider",
        ("AXIncrementor", _) => "spinner",
        ("AXWebArea", _) => "document",
        ("AXScrollArea", _) => "pane",
        ("AXToolbar", _) => "toolbar",
        ("AXWindow", _) | ("AXSheet", _) | ("AXDrawer", _) => "window",
        ("AXHeading", _) => "heading",
        ("AXProgressIndicator", _) => "progressbar",
        ("AXSplitter", _) => "separator",
        _ => "group",
    }
}

fn source() -> Result<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|()| anyhow!("could not create an event source"))
}

/// Types text exactly, whatever the keyboard layout, as Unicode keyboard events.
fn type_text(text: &str) -> Result<()> {
    let chars = text.chars().collect::<Vec<_>>();
    for chunk in chars.chunks(16) {
        crate::common::stopped()?;
        let chunk = chunk.iter().collect::<String>();
        for down in [true, false] {
            let event = CGEvent::new_keyboard_event(source()?, 0, down)
                .map_err(|()| anyhow!("could not create a key event"))?;
            event.set_string(&chunk);
            event.post(CGEventTapLocation::HID);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn click(x: i32, y: i32, button: &str) -> Result<()> {
    let point = CGPoint::new(f64::from(x), f64::from(y));
    let (down, up, which) = match button {
        "right" => (
            CGEventType::RightMouseDown,
            CGEventType::RightMouseUp,
            CGMouseButton::Right,
        ),
        _ => (
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
            CGMouseButton::Left,
        ),
    };
    let clicks = if button == "double" { 2 } else { 1 };
    let moved = CGEvent::new_mouse_event(source()?, CGEventType::MouseMoved, point, which)
        .map_err(|()| anyhow!("could not create a mouse event"))?;
    moved.post(CGEventTapLocation::HID);
    std::thread::sleep(Duration::from_millis(30));
    for count in 1..=clicks {
        for kind in [down, up] {
            let event = CGEvent::new_mouse_event(source()?, kind, point, which)
                .map_err(|()| anyhow!("could not create a mouse event"))?;
            event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, count);
            event.post(CGEventTapLocation::HID);
        }
    }
    Ok(())
}

fn scroll(x: i32, y: i32, lines: i32) -> Result<()> {
    click_free_move(x, y)?;
    let event = CGEvent::new_scroll_event(
        source()?,
        core_graphics::event::ScrollEventUnit::LINE,
        1,
        lines,
        0,
        0,
    )
    .map_err(|()| anyhow!("could not create a scroll event"))?;
    event.post(CGEventTapLocation::HID);
    let deadline = Instant::now() + Duration::from_millis(120);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// Moves the pointer without clicking, so a scroll lands on the element under it.
fn click_free_move(x: i32, y: i32) -> Result<()> {
    let point = CGPoint::new(f64::from(x), f64::from(y));
    CGEvent::new_mouse_event(
        source()?,
        CGEventType::MouseMoved,
        point,
        CGMouseButton::Left,
    )
    .map_err(|()| anyhow!("could not create a mouse event"))?
    .post(CGEventTapLocation::HID);
    Ok(())
}

/// Virtual key codes for named keys.
fn named(name: &str) -> Option<u16> {
    Some(match name {
        "enter" => 0x24,
        "tab" => 0x30,
        "space" => 0x31,
        "backspace" => 0x33,
        "escape" => 0x35,
        "delete" => 0x75,
        "home" => 0x73,
        "end" => 0x77,
        "pageup" => 0x74,
        "pagedown" => 0x79,
        "left" => 0x7B,
        "right" => 0x7C,
        "down" => 0x7D,
        "up" => 0x7E,
        "insert" => 0x72,
        "f1" => 0x7A,
        "f2" => 0x78,
        "f3" => 0x63,
        "f4" => 0x76,
        "f5" => 0x60,
        "f6" => 0x61,
        "f7" => 0x62,
        "f8" => 0x64,
        "f9" => 0x65,
        "f10" => 0x6D,
        "f11" => 0x67,
        "f12" => 0x6F,
        _ => return None,
    })
}

/// Virtual key codes for characters on a US layout, for shortcuts such as Command+S.
fn character(char: char) -> Option<u16> {
    let keys = [
        ('a', 0x00),
        ('s', 0x01),
        ('d', 0x02),
        ('f', 0x03),
        ('h', 0x04),
        ('g', 0x05),
        ('z', 0x06),
        ('x', 0x07),
        ('c', 0x08),
        ('v', 0x09),
        ('b', 0x0B),
        ('q', 0x0C),
        ('w', 0x0D),
        ('e', 0x0E),
        ('r', 0x0F),
        ('y', 0x10),
        ('t', 0x11),
        ('1', 0x12),
        ('2', 0x13),
        ('3', 0x14),
        ('4', 0x15),
        ('6', 0x16),
        ('5', 0x17),
        ('=', 0x18),
        ('9', 0x19),
        ('7', 0x1A),
        ('-', 0x1B),
        ('8', 0x1C),
        ('0', 0x1D),
        (']', 0x1E),
        ('o', 0x1F),
        ('u', 0x20),
        ('[', 0x21),
        ('i', 0x22),
        ('p', 0x23),
        ('l', 0x25),
        ('j', 0x26),
        ('\'', 0x27),
        ('k', 0x28),
        (';', 0x29),
        ('\\', 0x2A),
        (',', 0x2B),
        ('/', 0x2C),
        ('n', 0x2D),
        ('m', 0x2E),
        ('.', 0x2F),
        ('`', 0x32),
    ];
    keys.iter()
        .find(|(key, _)| *key == char)
        .map(|(_, code)| *code)
}
