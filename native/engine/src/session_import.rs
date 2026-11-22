//! Brings conversations from other coding agents on this computer into Cryzo, as Cursor and the
//! ChatGPT app do: Claude Code's transcripts in `~/.claude/projects` and Codex's sessions in
//! `~/.codex/sessions`. Each becomes a Cryzo session in the folder it was held in, with its
//! original times. Importing the same file again does nothing.
//!
//! Only what the user and the agent said and did is kept: the user's messages, the agent's
//! replies and reasoning summaries, and its tool calls with their results. Context the other
//! agent injected (environment notes, reminders, slash-command output) is left out.
use crate::service::Engine;
use crate::session_store::{id, tokens};
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// How far back to look, like Cursor and the ChatGPT app.
const RECENT: std::time::Duration = std::time::Duration::from_secs(30 * 86_400);
/// Tool results longer than this are cut: they're for reading back, not for the next model.
const RESULT_LIMIT: usize = 8_000;
const NAMESPACE: &str = "import";

pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/api/session/import", get(sources).post(import))
        .with_state(engine)
}

#[derive(Debug, Default)]
pub struct Conversation {
    pub source: &'static str,
    pub directory: String,
    pub title: String,
    pub created: u64,
    pub turns: Vec<Turn>,
}

#[derive(Debug)]
pub enum Turn {
    User {
        text: String,
        at: u64,
    },
    Agent {
        model: String,
        at: u64,
        parts: Vec<Part>,
    },
}

#[derive(Debug)]
pub enum Part {
    Text(String),
    Reasoning(String),
    Tool {
        id: String,
        name: String,
        input: Value,
        output: Option<String>,
        failed: bool,
    },
}

/// The conversations found on this computer, newest first, with whether each is already in Cryzo.
async fn sources(State(engine): State<Arc<Engine>>) -> ApiResult {
    let imported = imported(&engine);
    let found = candidates(&engine);
    let list = tokio::task::spawn_blocking(move || {
        found
            .into_iter()
            .filter_map(|(source, path, modified)| {
                // The start of a transcript names its folder and first message. Transcripts run
                // to hundreds of megabytes, so the whole file is only read on import.
                let conversation = read(source, &path, Some(PREVIEW))?;
                if scratch(&conversation.directory) {
                    return None;
                }
                Some(json!({
                    "source": source,
                    "file": path,
                    "title": conversation.title,
                    "directory": conversation.directory,
                    "exists": Path::new(&conversation.directory).is_dir(),
                    "updated": modified,
                }))
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(invalid)?;
    let list: Vec<Value> = list
        .into_iter()
        .map(|mut item| {
            let file = item["file"].as_str().unwrap_or_default().to_owned();
            if let Some(session) = imported.iter().find(|(key, _)| *key == file) {
                item["imported"] = json!(true);
                item["session"] = session.1.clone();
                // The other agent kept going after the import, as an open conversation does.
                item["stale"] = json!(
                    item["updated"].as_u64().unwrap_or(0)
                        > session.1["updated"].as_u64().unwrap_or(0) + 60_000
                );
            } else {
                item["imported"] = json!(false);
            }
            item
        })
        .collect();
    Ok(Json(json!({"conversations":list})))
}

/// Imports the chosen files. Each one becomes a session. Files already imported are skipped,
/// unless `update` asks for their newer transcript.
async fn import(State(engine): State<Arc<Engine>>, Json(input): Json<Value>) -> ApiResult {
    tokio::task::spawn_blocking(move || import_files(&engine, &input))
        .await
        .map(Json)
        .map_err(invalid)
}

fn import_files(engine: &Engine, input: &Value) -> Value {
    let chosen: Vec<String> = input["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|file| file.as_str().map(str::to_owned))
        .collect();
    // Only files the scan found can be imported, so the request can't name arbitrary paths.
    let found = candidates(engine);
    let mut sessions = Vec::new();
    let mut skipped = Vec::new();
    for file in chosen {
        let Some((source, path, _)) = found
            .iter()
            .find(|(_, path, _)| path.to_string_lossy() == file)
        else {
            skipped.push(json!({"file":file,"reason":"not found"}));
            continue;
        };
        if let Some((_, earlier)) = imported(engine).into_iter().find(|(key, _)| *key == file) {
            if input["update"] != true {
                skipped.push(json!({"file":file,"reason":"already imported"}));
                continue;
            }
            // Updating brings in the whole transcript again. The earlier copy goes unless the user
            // continued it in Cryzo, in which case both stay.
            if earlier["continued"] != true
                && let Some(id) = earlier["id"].as_str()
                && let Err(error) = engine.record(id, "session.deleted", json!({}))
            {
                skipped.push(json!({"file":file,"reason":error.to_string()}));
                continue;
            }
        }
        match save(engine, source, path) {
            Ok(session) => sessions.push(session),
            Err(error) => skipped.push(json!({"file":file,"reason":error.to_string()})),
        }
    }
    json!({"sessions":sessions,"skipped":skipped})
}

/// Writes one conversation as a Cryzo session and returns its summary.
fn save(engine: &Engine, source: &'static str, path: &Path) -> anyhow::Result<Value> {
    let conversation =
        read(source, path, None).ok_or_else(|| anyhow::anyhow!("nothing to import"))?;
    let directory = PathBuf::from(&conversation.directory);
    anyhow::ensure!(directory.is_dir(), "its folder no longer exists");
    let project = engine.store.desktop_project(&directory)?;
    let directory = directory.canonicalize()?;
    let session = id("ses");
    let updated = conversation
        .turns
        .iter()
        .map(|turn| match turn {
            Turn::User { at, .. } | Turn::Agent { at, .. } => *at,
        })
        .max()
        .unwrap_or(conversation.created);
    let mut info = json!({
        "id": session,
        "projectID": project["id"],
        "title": conversation.title,
        "agent": "build",
        "location": {"directory": directory.to_string_lossy().trim_start_matches(r"\\?\")},
        "time": {"created": conversation.created, "updated": updated},
        "cost": 0,
        "tokens": tokens(),
    });
    // Continuing it uses the user's own model, as a new session would.
    if let Some(model) = crate::desktop_models::models(engine)?
        .iter()
        .find(|model| model["enabled"] == true)
    {
        info["model"] = json!({"id":model["id"],"providerID":model["providerID"]});
    }
    let imported = summary(&info);
    let event = engine.store.desktop_import(info, events(&conversation))?;
    engine.store.desktop_put_meta(
        NAMESPACE,
        &path.to_string_lossy(),
        Some(&json!({"session":session,"source":source})),
    )?;
    engine.emit(event);
    Ok(imported)
}

/// The session events a live run would have written for this conversation.
pub fn events(conversation: &Conversation) -> Vec<(&'static str, Value, u64)> {
    let mut events = Vec::new();
    for turn in &conversation.turns {
        match turn {
            Turn::User { text, at } => {
                let inbox = id("msg");
                events.push((
                    "session.inbox.enqueued",
                    json!({"inboxID":inbox,"item":{"type":"user","payload":{"text":text},"delivery":"steer"}}),
                    *at,
                ));
                events.push(("session.inbox.delivered", json!({"inboxID":inbox}), *at));
            }
            Turn::Agent { model, at, parts } => {
                let message = id("msg");
                let model =
                    json!({"id":model,"providerID":format!("imported-{}",conversation.source)});
                events.push((
                    "session.step.started",
                    json!({"assistantMessageID":message,"agent":"build","model":model}),
                    *at,
                ));
                for (ordinal, part) in parts.iter().enumerate() {
                    let on = |data: Value| {
                        let mut data = data;
                        data["assistantMessageID"] = json!(message);
                        data
                    };
                    match part {
                        Part::Text(text) => {
                            events.push(("session.text.started", on(json!({})), *at));
                            events.push((
                                "session.text.ended",
                                on(json!({"ordinal":ordinal,"text":text})),
                                *at,
                            ));
                        }
                        Part::Reasoning(text) => {
                            events.push(("session.reasoning.started", on(json!({})), *at));
                            events.push((
                                "session.reasoning.ended",
                                on(json!({"ordinal":ordinal,"text":text})),
                                *at,
                            ));
                        }
                        Part::Tool {
                            id,
                            name,
                            input,
                            output,
                            failed,
                        } => {
                            events.push((
                                "session.tool.input.started",
                                on(json!({"id":id,"name":name})),
                                *at,
                            ));
                            events.push((
                                "session.tool.input.ended",
                                on(json!({"id":id,"text":input.to_string()})),
                                *at,
                            ));
                            events.push((
                                "session.tool.called",
                                on(json!({"id":id,"input":input,"executed":true})),
                                *at,
                            ));
                            let output = output.clone().unwrap_or_default();
                            events.push(if *failed {
                                (
                                    "session.tool.failed",
                                    on(json!({"id":id,"executed":true,"error":{"type":"ToolError","message":output}})),
                                    *at,
                                )
                            } else {
                                (
                                    "session.tool.success",
                                    on(json!({"id":id,"executed":true,"content":[{"type":"text","text":output}]})),
                                    *at,
                                )
                            });
                        }
                    }
                }
                events.push((
                    "session.step.ended",
                    json!({"assistantMessageID":message,"finish":"stop","cost":0,"tokens":tokens()}),
                    *at,
                ));
            }
        }
    }
    events
}

/// The transcripts the user can import: Claude Code runs that Cryzo itself started for a Claude
/// plan are already Cryzo sessions, so they're left out.
fn candidates(engine: &Engine) -> Vec<(&'static str, PathBuf, u64)> {
    let own: Vec<String> = engine
        .store
        .desktop_meta("claude-code")
        .unwrap_or_default()
        .iter()
        .filter_map(|run| run["conversation"].as_str().map(str::to_owned))
        .collect();
    files()
        .into_iter()
        .filter(|(_, path, _)| {
            !path
                .file_stem()
                .is_some_and(|stem| own.iter().any(|id| *id == stem.to_string_lossy()))
        })
        .collect()
}

/// A folder in the system's temporary files, where tools run throwaway sessions.
fn scratch(directory: &str) -> bool {
    let temp = std::env::temp_dir();
    let temp = temp.to_string_lossy();
    let temp = temp.trim_end_matches(['\\', '/']).to_lowercase();
    directory.to_lowercase().starts_with(&temp)
}

/// The transcript files of both agents changed in the last 30 days, newest first.
fn files() -> Vec<(&'static str, PathBuf, u64)> {
    let Some(home) =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
    else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for folder in std::fs::read_dir(home.join(".claude/projects"))
        .into_iter()
        .flatten()
        .flatten()
    {
        for file in std::fs::read_dir(folder.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = file.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            {
                found.push(("claude-code", path));
            }
        }
    }
    let mut pending = vec![home.join(".codex/sessions")];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if entry.file_name().to_string_lossy().starts_with("rollout-")
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
            {
                found.push(("codex", path));
            }
        }
    }
    let cutoff = std::time::SystemTime::now() - RECENT;
    let mut recent: Vec<_> = found
        .into_iter()
        .filter_map(|(source, path)| {
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            (modified >= cutoff).then(|| {
                let millis = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                (source, path, millis)
            })
        })
        .collect();
    recent.sort_by_key(|item| std::cmp::Reverse(item.2));
    recent
}

/// How much of a transcript the list reads.
const PREVIEW: u64 = 2 * 1024 * 1024;

/// A transcript, or only its first `limit` bytes (up to the last whole line).
fn read(source: &'static str, path: &Path, limit: Option<u64>) -> Option<Conversation> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(limit.unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .ok()?;
    if limit.is_some_and(|limit| bytes.len() as u64 >= limit) {
        bytes.truncate(bytes.iter().rposition(|byte| *byte == b'\n').unwrap_or(0));
    }
    let text = String::from_utf8_lossy(&bytes);
    let conversation = match source {
        "claude-code" => claude_code(&text),
        _ => codex(&text),
    };
    conversation.filter(|conversation| {
        !conversation.directory.is_empty()
            && conversation
                .turns
                .iter()
                .any(|turn| matches!(turn, Turn::User { .. }))
    })
}

/// A Claude Code transcript: one JSON object per line. Assistant replies arrive one content block
/// per line; tool results come back inside the following user lines, matched by `tool_use_id`.
pub fn claude_code(text: &str) -> Option<Conversation> {
    let mut conversation = Conversation {
        source: "claude-code",
        ..Conversation::default()
    };
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry["isSidechain"] == true || entry["isMeta"] == true {
            continue;
        }
        let at = entry["timestamp"]
            .as_str()
            .and_then(crate::market::epoch_ms)
            .map_or(conversation.created, |at| at as u64);
        if conversation.directory.is_empty()
            && let Some(cwd) = entry["cwd"].as_str()
        {
            conversation.directory = cwd.to_owned();
        }
        match entry["type"].as_str() {
            Some("user") => {
                let content = &entry["message"]["content"];
                if let Some(text) = content.as_str() {
                    user(&mut conversation, text, at);
                    continue;
                }
                for block in content.as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => user(
                            &mut conversation,
                            block["text"].as_str().unwrap_or_default(),
                            at,
                        ),
                        Some("tool_result") => {
                            let output = match &block["content"] {
                                Value::String(text) => text.clone(),
                                content => content
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(|part| part["text"].as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                            };
                            finish(
                                &mut conversation,
                                block["tool_use_id"].as_str().unwrap_or_default(),
                                output,
                                block["is_error"] == true,
                            );
                        }
                        _ => {}
                    }
                }
            }
            Some("assistant") => {
                let model = entry["message"]["model"].as_str().unwrap_or("claude");
                for block in entry["message"]["content"].as_array().into_iter().flatten() {
                    let part = match block["type"].as_str() {
                        Some("text") => {
                            Part::Text(block["text"].as_str().unwrap_or_default().to_owned())
                        }
                        Some("thinking") => Part::Reasoning(
                            block["thinking"].as_str().unwrap_or_default().to_owned(),
                        ),
                        Some("tool_use") => Part::Tool {
                            id: block["id"].as_str().unwrap_or_default().to_owned(),
                            name: block["name"].as_str().unwrap_or("tool").to_owned(),
                            input: block["input"].clone(),
                            output: None,
                            failed: false,
                        },
                        _ => continue,
                    };
                    agent(&mut conversation, model, part, at);
                }
            }
            _ => {}
        }
    }
    Some(conversation)
}

/// A Codex session: a `session_meta` line with the folder, then `response_item` lines for each
/// message, reasoning summary, tool call and tool result.
pub fn codex(text: &str) -> Option<Conversation> {
    let mut conversation = Conversation {
        source: "codex",
        ..Conversation::default()
    };
    let mut model = "codex".to_owned();
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let at = entry["timestamp"]
            .as_str()
            .and_then(crate::market::epoch_ms)
            .map_or(conversation.created, |at| at as u64);
        let payload = &entry["payload"];
        match entry["type"].as_str() {
            Some("session_meta") => {
                conversation.directory = payload["cwd"].as_str().unwrap_or_default().to_owned();
            }
            Some("turn_context") => {
                if let Some(name) = payload["model"].as_str() {
                    model = name.to_owned();
                }
            }
            Some("response_item") => match payload["type"].as_str() {
                Some("message") => {
                    let text = payload["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|part| part["text"].as_str())
                        .filter(|text| !injected(text))
                        .collect::<Vec<_>>()
                        .join("\n");
                    match payload["role"].as_str() {
                        Some("user") => user(&mut conversation, &text, at),
                        Some("assistant") if !text.trim().is_empty() => {
                            agent(&mut conversation, &model, Part::Text(text), at)
                        }
                        _ => {}
                    }
                }
                Some("reasoning") => {
                    let summary = payload["summary"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|part| part["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !summary.trim().is_empty() {
                        agent(&mut conversation, &model, Part::Reasoning(summary), at);
                    }
                }
                Some("function_call" | "custom_tool_call") => {
                    let input = payload["arguments"]
                        .as_str()
                        .and_then(|arguments| serde_json::from_str(arguments).ok())
                        .unwrap_or_else(|| json!({"input":payload["input"].as_str().or(payload["arguments"].as_str()).unwrap_or_default()}));
                    agent(
                        &mut conversation,
                        &model,
                        Part::Tool {
                            id: payload["call_id"].as_str().unwrap_or_default().to_owned(),
                            name: payload["name"].as_str().unwrap_or("tool").to_owned(),
                            input,
                            output: None,
                            failed: false,
                        },
                        at,
                    );
                }
                Some("function_call_output" | "custom_tool_call_output") => {
                    let output = match &payload["output"] {
                        Value::String(text) => text.clone(),
                        output => output
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|part| part["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    };
                    finish(
                        &mut conversation,
                        payload["call_id"].as_str().unwrap_or_default(),
                        output,
                        false,
                    );
                }
                _ => {}
            },
            _ => {}
        }
    }
    Some(conversation)
}

/// Context an agent adds to the user's messages by itself, such as `<environment_context>` or
/// `<system-reminder>` blocks and attached-image markers.
fn injected(text: &str) -> bool {
    let text = text.trim();
    let Some(rest) = text.strip_prefix('<') else {
        return false;
    };
    let tag: String = rest
        .chars()
        .take_while(|char| char.is_ascii_alphanumeric() || *char == '_' || *char == '-')
        .collect();
    !tag.is_empty() && (text.ends_with(&format!("</{tag}>")) || tag == "image")
        || text.starts_with("</")
        || tag.starts_with("command-")
        || tag.starts_with("local-command")
}

fn user(conversation: &mut Conversation, text: &str, at: u64) {
    let text = text.trim();
    if text.is_empty() || injected(text) || text.starts_with("Caveat: The messages below") {
        return;
    }
    if conversation.title.is_empty() {
        conversation.title = title(text);
        conversation.created = at;
    }
    if conversation.created == 0 {
        conversation.created = at;
    }
    conversation.turns.push(Turn::User {
        text: text.to_owned(),
        at,
    });
}

/// The first line of the message with markup such as `<pasted_content id="…">` taken out.
fn title(text: &str) -> String {
    let mut plain = String::new();
    let mut tag = false;
    for char in text.chars() {
        match char {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => plain.push(char),
            _ => {}
        }
    }
    let line = plain
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(text);
    line.chars().take(80).collect()
}

/// Adds to the agent's reply since the last user message, starting one when needed.
fn agent(conversation: &mut Conversation, model: &str, part: Part, at: u64) {
    if conversation.created == 0 {
        conversation.created = at;
    }
    if let Some(Turn::Agent { parts, .. }) = conversation.turns.last_mut() {
        parts.push(part);
        return;
    }
    conversation.turns.push(Turn::Agent {
        model: model.to_owned(),
        at,
        parts: vec![part],
    });
}

/// Records a tool's result on the call it answers.
fn finish(conversation: &mut Conversation, call: &str, output: String, error: bool) {
    let output = if output.len() > RESULT_LIMIT {
        let mut end = RESULT_LIMIT;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &output[..end])
    } else {
        output
    };
    for turn in conversation.turns.iter_mut().rev() {
        let Turn::Agent { parts, .. } = turn else {
            continue;
        };
        for part in parts.iter_mut().rev() {
            if let Part::Tool {
                id,
                output: slot,
                failed,
                ..
            } = part
                && id == call
            {
                *slot = Some(output);
                *failed = error;
                return;
            }
        }
    }
}

/// Files already imported, by path, with their session as `{id, directory, title}`. A session the
/// user deleted no longer counts, so its file can be imported again.
fn imported(engine: &Engine) -> Vec<(String, Value)> {
    engine
        .store
        .desktop_meta_entries(NAMESPACE)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(file, entry)| {
            let state = engine
                .store
                .desktop_state(entry["session"].as_str()?)
                .ok()?;
            if state["deleted"] == true {
                return None;
            }
            let mut session = summary(&state["info"]);
            let messages = state["messages"].as_array().cloned().unwrap_or_default();
            // When the chat last changed in Cryzo, and whether the user carried it on here with
            // another model: then updating it must not throw that work away.
            session["updated"] = messages
                .iter()
                .filter_map(|message| message["time"]["created"].as_u64())
                .max()
                .map_or(Value::Null, |at| json!(at));
            session["continued"] = json!(messages.iter().any(|message| {
                message["type"] == "assistant"
                    && !message["model"]["providerID"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("imported-"))
            }));
            Some((file, session))
        })
        .collect()
}

/// What the app needs to show and open an imported session.
fn summary(info: &Value) -> Value {
    json!({"id":info["id"],"directory":info["location"]["directory"],"title":info["title"]})
}

type ApiResult = std::result::Result<Json<Value>, (StatusCode, Json<Value>)>;

fn invalid(error: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"_tag":"InvalidRequestError","message":format!("{error:#}")})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claude_code_transcript_keeps_the_conversation_and_pairs_tool_results() {
        let transcript = [
            r#"{"type":"user","cwd":"C:\\work\\app","timestamp":"2026-11-20T10:00:00.000Z","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
            r#"{"type":"user","cwd":"C:\\work\\app","timestamp":"2026-11-20T10:00:01.000Z","message":{"role":"user","content":"Fix the login bug\nIt crashes on submit"}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-20T10:00:02.000Z","message":{"id":"m1","model":"claude-opus-5-5","content":[{"type":"thinking","thinking":"Look at the form","signature":"x"}]}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-20T10:00:03.000Z","message":{"id":"m1","model":"claude-opus-5-5","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/login.ts"}}]}}"#,
            r#"{"type":"user","timestamp":"2026-11-20T10:00:04.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"export function login() {}"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"a subagent's work"}]}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-20T10:00:05.000Z","message":{"id":"m2","model":"claude-opus-5-5","content":[{"type":"text","text":"Fixed it."}]}}"#,
            "not json",
        ]
        .join("\n");
        let conversation = claude_code(&transcript).unwrap();
        assert_eq!(conversation.directory, r"C:\work\app");
        assert_eq!(conversation.title, "Fix the login bug");
        assert_eq!(
            conversation.created,
            crate::market::epoch_ms("2026-11-20T10:00:01Z").unwrap() as u64
        );
        assert_eq!(
            conversation.turns.len(),
            2,
            "one user message and one reply"
        );
        let Turn::Agent { parts, model, .. } = &conversation.turns[1] else {
            panic!("expected the agent's reply");
        };
        assert_eq!(model, "claude-opus-5-5");
        assert!(matches!(&parts[0], Part::Reasoning(text) if text == "Look at the form"));
        assert!(
            matches!(&parts[1], Part::Tool { name, output: Some(output), failed: false, .. } if name == "Read" && output == "export function login() {}")
        );
        assert!(matches!(&parts[2], Part::Text(text) if text == "Fixed it."));
    }

    #[test]
    fn titles_leave_out_markup() {
        assert_eq!(
            title("<pasted_content id=\"a01c\">\nDictation isn't accurate\nmore</pasted_content>"),
            "Dictation isn't accurate"
        );
        assert_eq!(title("Fix the build"), "Fix the build");
    }

    #[test]
    fn a_codex_session_drops_injected_context_and_keeps_tool_calls() {
        let rollout = [
            r#"{"type":"session_meta","timestamp":"2026-11-20T10:00:00Z","payload":{"cwd":"/home/me/app"}}"#,
            r#"{"type":"turn_context","payload":{"model":"gpt-6.2"}}"#,
            r#"{"type":"response_item","timestamp":"2026-11-20T10:00:01Z","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"system rules"}]}}"#,
            r#"{"type":"response_item","timestamp":"2026-11-20T10:00:01Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n<cwd>/home/me/app</cwd>\n</environment_context>"}]}}"#,
            r#"{"type":"response_item","timestamp":"2026-11-20T10:00:02Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Add a dark mode"}]}}"#,
            r#"{"type":"response_item","timestamp":"2026-11-20T10:00:03Z","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"Check the theme"}],"encrypted_content":"x"}}"#,
            r#"{"type":"response_item","timestamp":"2026-11-20T10:00:04Z","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"ls\"}","call_id":"c1"}}"#,
            r#"{"type":"response_item","timestamp":"2026-11-20T10:00:05Z","payload":{"type":"function_call_output","call_id":"c1","output":[{"type":"input_text","text":"src"}]}}"#,
            r#"{"type":"response_item","timestamp":"2026-11-20T10:00:06Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Added it."}]}}"#,
        ]
        .join("\n");
        let conversation = codex(&rollout).unwrap();
        assert_eq!(conversation.directory, "/home/me/app");
        assert_eq!(conversation.title, "Add a dark mode");
        assert_eq!(conversation.turns.len(), 2);
        let Turn::Agent { parts, model, .. } = &conversation.turns[1] else {
            panic!("expected the agent's reply");
        };
        assert_eq!(model, "gpt-6.2");
        assert_eq!(parts.len(), 3);
        assert!(
            matches!(&parts[1], Part::Tool { input, output: Some(output), .. } if input["cmd"] == "ls" && output == "src")
        );
    }

    #[test]
    fn an_imported_conversation_reads_back_as_a_session_with_its_times() {
        let folder = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&folder.path().join("db.sqlite")).unwrap();
        let conversation = Conversation {
            source: "codex",
            directory: "/app".into(),
            title: "Add a dark mode".into(),
            created: 1_000,
            turns: vec![
                Turn::User {
                    text: "Add a dark mode".into(),
                    at: 1_000,
                },
                Turn::Agent {
                    model: "gpt-6.2".into(),
                    at: 2_000,
                    parts: vec![
                        Part::Tool {
                            id: "c1".into(),
                            name: "exec_command".into(),
                            input: json!({"cmd":"ls"}),
                            output: Some("denied".into()),
                            failed: true,
                        },
                        Part::Text("Done.".into()),
                    ],
                },
            ],
        };
        let info = json!({"id":"ses_import","title":"Add a dark mode","agent":"build","location":{"directory":"/app"},"time":{"created":1_000,"updated":2_000},"cost":0,"tokens":tokens()});
        let created = store.desktop_import(info, events(&conversation)).unwrap();
        assert_eq!(created["created"], 1_000);
        let state = store.desktop_state("ses_import").unwrap();
        let messages = state["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["type"], "user");
        assert_eq!(messages[0]["text"], "Add a dark mode");
        assert_eq!(messages[0]["time"]["created"], 1_000);
        assert_eq!(messages[1]["model"]["providerID"], "imported-codex");
        assert_eq!(messages[1]["content"][0]["state"]["status"], "error");
        assert_eq!(messages[1]["content"][1]["text"], "Done.");
        assert_eq!(state["status"], "idle");
        assert!(state["inbox"].as_array().unwrap().is_empty());
    }
}
