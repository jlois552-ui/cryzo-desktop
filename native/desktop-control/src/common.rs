//! Pieces every platform's desktop control shares: the stop switch, how changes between two
//! snapshots are shown, and text clean-up. Compiled on every OS.

use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, bail};

use crate::ax::invisible;

/// Set by the global stop hotkey. Once set, every acting call fails until the process restarts.
pub(crate) static STOPPED: AtomicBool = AtomicBool::new(false);
/// Elements that disappeared are listed one by one up to this many, then only counted.
const GONE: usize = 8;

/// A rectangle in physical screen pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[cfg(windows)]
impl From<windows::Win32::Foundation::RECT> for Rect {
    fn from(rect: windows::Win32::Foundation::RECT) -> Self {
        Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

/// The overlay's Stop button: the same as pressing Ctrl+Alt+Esc.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn stop() {
    STOPPED.store(true, Ordering::SeqCst);
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_stopped() -> bool {
    STOPPED.load(Ordering::SeqCst)
}

pub(crate) fn stopped() -> Result<()> {
    if STOPPED.load(Ordering::SeqCst) {
        bail!(
            "The user stopped desktop control (the Stop button or Ctrl+Alt+Esc). Don't retry; it resumes when the user sends a new message."
        );
    }
    Ok(())
}

/// How the overlay names an action, as in `Clicking button "Save"`.
pub(crate) fn verb(action: &str) -> &str {
    match action {
        "click" => "Clicking",
        "double_click" => "Double-clicking",
        "right_click" => "Right-clicking",
        "type" => "Typing into",
        "toggle" => "Toggling",
        "select" | "add_to_selection" => "Selecting",
        "expand" => "Expanding",
        "collapse" => "Collapsing",
        "scroll_into_view" => "Scrolling to",
        "scroll_down" => "Scrolling down",
        "scroll_up" => "Scrolling up",
        "focus" => "Focusing",
        _ => action,
    }
}

/// The element ID a snapshot line starts with, such as `e12` from `[e12] button "Save"`.
pub(crate) fn line_id(line: &str) -> Option<String> {
    line.strip_prefix('[')?
        .split_once(']')
        .map(|(id, _)| id.to_string())
}

/// One line for an acting step in the middle of a batch: its result line, window notes such as
/// "closed" or the new foreground, and how many elements changed.
pub(crate) fn summary(text: &str) -> String {
    let changed = text
        .lines()
        .filter(|line| {
            ["~ ", "+ ", "- ", "  "]
                .iter()
                .any(|mark| line.starts_with(mark))
        })
        .count();
    let notes = text
        .lines()
        .filter(|line| {
            !["~ ", "+ ", "- ", "  "]
                .iter()
                .any(|mark| line.starts_with(mark))
        })
        .map(|line| line.trim_end_matches(" changed:"))
        .collect::<Vec<_>>()
        .join("; ");
    match changed {
        0 => notes,
        _ => format!("{notes} ({changed} lines changed)"),
    }
}

/// `~` marks an element whose line changed, `+` a new one and `-` one that is gone. Long runs of
/// gone elements (a view switched away) are only counted, since the model rarely needs them.
pub(crate) fn changes(header: &str, previous: &[String], now: &[String]) -> String {
    let id = line_id;
    let before = previous
        .iter()
        .map(|line| line.trim())
        .collect::<HashSet<_>>();
    let after = now.iter().map(|line| line.trim()).collect::<HashSet<_>>();
    let ids_before = before
        .iter()
        .filter_map(|line| id(line))
        .collect::<HashSet<_>>();
    let ids_after = after
        .iter()
        .filter_map(|line| id(line))
        .collect::<HashSet<_>>();
    let gone = previous
        .iter()
        .map(|line| line.trim())
        .filter(|line| !after.contains(line))
        .filter(|line| id(line).is_none_or(|id| !ids_after.contains(&id)))
        .collect::<Vec<_>>();
    let gone = if gone.len() > GONE {
        vec![format!("- {} elements gone", gone.len())]
    } else {
        gone.iter().map(|line| format!("- {line}")).collect()
    };
    let lines = gone
        .into_iter()
        .chain(
            now.iter()
                .map(|line| line.trim())
                .filter(|line| !before.contains(line))
                .map(|line| match id(line) {
                    Some(id) if ids_before.contains(&id) => format!("~ {line}"),
                    _ => format!("+ {line}"),
                }),
        )
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return format!("{header}: no visible change");
    }
    format!("{header} changed:\n{}", lines.join("\n"))
}

/// One line of screen text: no icon-font glyphs or invisible marks, single spaces, at most 120
/// characters.
pub(crate) fn clean(text: &str) -> String {
    // Icon fonts put glyphs in the Unicode private use area; they carry no meaning as text.
    let text = text
        .chars()
        .filter(|c| !('\u{e000}'..='\u{f8ff}').contains(c) && !invisible(*c))
        .collect::<String>();
    clip(&text.split_whitespace().collect::<Vec<_>>().join(" "), 120)
}

pub(crate) fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max).collect::<String>())
}
