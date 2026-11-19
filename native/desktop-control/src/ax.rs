//! Pages as Chrome's accessibility tree, in the compact lines desktop snapshots use
//! (`[b12] link "Download for Windows"`). Cryzo's Chrome window and the app's browser pane both read
//! pages this way, the way browser agents such as Claude in Chrome do.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::Value;

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

/// An element a snapshot line names, found again by its DOM backend node ID.
#[derive(Clone)]
pub struct Element {
    pub backend: i64,
    pub role: String,
    pub name: String,
}

/// The lines of an `Accessibility.getFullAXTree` result, at most `max_lines`, and the elements they
/// name by ID. `id` gives each DOM backend node its line ID.
pub fn snapshot(
    tree: &Value,
    id: &mut dyn FnMut(i64) -> String,
    max_lines: usize,
) -> Result<(Vec<String>, HashMap<String, Element>)> {
    let nodes = tree["nodes"].as_array().context("no accessibility tree")?;
    let by_id = nodes
        .iter()
        .filter_map(|node| Some((node["nodeId"].as_str()?, node)))
        .collect::<HashMap<_, _>>();
    let root = nodes.first().context("empty page")?;
    let mut lines = Vec::new();
    let mut elements = HashMap::new();
    visit(
        root,
        &by_id,
        0,
        "",
        &mut Walk {
            id,
            lines: &mut lines,
            elements: &mut elements,
            max_lines,
        },
    );
    Ok((lines, elements))
}

struct Walk<'a> {
    id: &'a mut dyn FnMut(i64) -> String,
    lines: &'a mut Vec<String>,
    elements: &'a mut HashMap<String, Element>,
    max_lines: usize,
}

fn visit(
    node: &Value,
    by_id: &HashMap<&str, &Value>,
    depth: usize,
    parent_name: &str,
    walk: &mut Walk,
) {
    if walk.lines.len() >= walk.max_lines {
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
            let id = (walk.id)(backend);
            let role = if role == "StaticText" { "text" } else { role };
            walk.lines.push(
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
            walk.elements.insert(
                id,
                Element {
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
            visit(child, by_id, next, &child_parent, walk);
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

fn clean(text: &str) -> String {
    let text = text.chars().filter(|c| !invisible(*c)).collect::<String>();
    clip(&text.split_whitespace().collect::<Vec<_>>().join(" "), 120)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max).collect::<String>())
}

/// A name or value in quotes, escaped like a JSON string: backslashes, quotes and line breaks
/// (as \n), nothing else. Debug formatting would turn every non-ASCII letter into a \u{…} escape
/// the model then has to decode.
pub fn quote(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

/// Zero-width and text-direction marks, which Windows puts around dates and numbers in some
/// locales. They are invisible on screen and only confuse matching.
pub fn invisible(c: char) -> bool {
    matches!(c, '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

#[cfg(test)]
mod tests {
    use super::snapshot;
    use serde_json::json;

    #[test]
    fn a_tree_reads_as_lines_with_ids_for_named_and_interactive_elements() {
        let tree = json!({"nodes":[
            {"nodeId":"1","role":{"value":"RootWebArea"},"name":{"value":"Shop"},"childIds":["2","3","5"]},
            {"nodeId":"2","role":{"value":"heading"},"name":{"value":"Sign  in"},"backendDOMNodeId":10,"childIds":["4"]},
            {"nodeId":"4","role":{"value":"StaticText"},"name":{"value":"Sign in"},"backendDOMNodeId":11},
            {"nodeId":"3","role":{"value":"textbox"},"name":{"value":""},"backendDOMNodeId":12,"properties":[{"name":"focused","value":{"value":true}}]},
            {"nodeId":"5","role":{"value":"generic"},"name":{"value":""},"backendDOMNodeId":13},
        ]});
        let (lines, elements) = snapshot(&tree, &mut |backend| format!("e{backend}"), 100).unwrap();
        assert_eq!(
            lines,
            vec![
                "  [e10] heading \"Sign in\"",
                "  [e12] textbox \"\" focused"
            ]
        );
        assert_eq!(elements["e12"].backend, 12);
        assert_eq!(elements["e10"].role, "heading");
    }
}
