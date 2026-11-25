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
pub(crate) const OUTCOME_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

pub(crate) fn outcome_query<'a>(
    args: &'a serde_json::Value,
    action: &str,
) -> Result<Option<&'a str>> {
    let Some(value) = args.get("waitFor") else {
        return Ok(None);
    };
    let Some(query) = value
        .as_str()
        .map(str::trim)
        .filter(|query| !query.is_empty() && !query.contains(['\r', '\n']))
    else {
        bail!("waitFor needs a nonempty element name or snapshot line on one line");
    };
    if action == "read" {
        bail!("waitFor requires an action that can change the window");
    }
    Ok(Some(query))
}

pub(crate) fn outcome_matches(lines: &[String], truncated: bool, query: &str) -> Option<usize> {
    if truncated
        || lines.is_empty()
        || lines.iter().any(|line| {
            let line = line.trim_start();
            line.starts_with("…") || line.starts_with("(could not read this window")
        })
    {
        return None;
    }
    Some(
        lines
            .iter()
            .filter(|line| {
                let Some((_, body)) = line
                    .trim_start()
                    .strip_prefix("[e")
                    .and_then(|line| line.split_once("] "))
                else {
                    return false;
                };
                let body = body.strip_suffix(" focused").unwrap_or(body);
                if body == query && !body.contains('…') {
                    return true;
                }
                let Some((_, attributes)) = body.split_once(' ') else {
                    return false;
                };
                serde_json::Deserializer::from_str(attributes)
                    .into_iter::<String>()
                    .next()
                    .is_some_and(|name| name.is_ok_and(|name| name == query))
            })
            .count(),
    )
}

pub(crate) fn outcome_note(outcome: Option<(&str, bool)>, matched: bool) -> &'static str {
    match (outcome, matched) {
        (Some(_), true) => "\nOutcome observed.",
        (Some((_, false)), _) => {
            "\nOutcome could not be verified as new. Returned the normal window update."
        }
        (Some(_), false) => {
            "\nOutcome was not observed. Returned the normal window update; the action was already sent."
        }
        (None, _) => "",
    }
}

#[cfg(test)]
mod outcome_tests {
    use super::outcome_matches;

    #[test]
    fn outcomes_match_exact_names_and_states() {
        let lines = [
            r#"  [e1] text "Draft saved""#.into(),
            r#"  [e2] checkbox "Reviewed" checked focused"#.into(),
            r#"  [e3] edit "Notes" value="One\nTwo""#.into(),
        ];
        assert_eq!(outcome_matches(&lines, false, "Draft saved"), Some(1));
        assert_eq!(outcome_matches(&lines, false, "saved"), Some(0));
        assert_eq!(
            outcome_matches(&lines, false, r#"checkbox "Reviewed" checked"#),
            Some(1)
        );
        assert_eq!(
            outcome_matches(&lines, false, r#"checkbox "Reviewed" unchecked"#),
            Some(0)
        );
        assert_eq!(
            outcome_matches(&lines, false, r#"edit "Notes" value="One\nTwo""#),
            Some(1)
        );
    }

    #[test]
    fn duplicate_outcomes_remain_ambiguous_and_clipped_values_do_not_match() {
        let lines = [
            r#"[e1] text "Draft saved""#.into(),
            r#"[e2] text "Draft saved""#.into(),
            r#"[e3] edit "Notes" value="One…""#.into(),
        ];
        assert_eq!(outcome_matches(&lines, false, "Draft saved"), Some(2));
        assert_eq!(
            outcome_matches(&lines, false, r#"edit "Notes" value="One…""#),
            Some(0)
        );
    }

    #[test]
    fn incomplete_observations_cannot_confirm_an_outcome() {
        assert_eq!(outcome_matches(&[], false, "Saved"), None);
        let lines = [r#"[e1] text "Saved""#.into(), "  … 20 more".into()];
        assert_eq!(outcome_matches(&lines, false, "Saved"), None);
        assert_eq!(outcome_matches(&lines[..1], true, "Saved"), None);
    }
}

/// A rectangle in physical screen pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
