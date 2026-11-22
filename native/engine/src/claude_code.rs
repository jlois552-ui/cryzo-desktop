//! Claude plans through the user's own Claude Code, the way Dyad does it. Cryzo starts the
//! unmodified `claude` program the user installed and signed in to, with its built-in tools turned
//! off: Claude thinks and writes, and every tool call comes back to Cryzo through a local MCP server,
//! so approvals, diffs, the preview and computer use work as they do with any other model.
//! Cryzo never reads, stores or relays Claude credentials; signing in is Claude Code's own
//! `claude auth login`.
use crate::{service::Engine, session_store, tools::Workspace};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    process::Stdio,
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{Mutex, mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

/// The connection method on the Anthropic integration.
pub const METHOD: &str = "claude-code";
/// Models a Claude plan serves through Claude Code, where the catalog has them.
pub const MODELS: &[&str] = &[
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "claude-haiku-5-5",
    "claude-fable-5-1",
];
/// Cryzo's tools as Claude Code names them: `mcp__<server>__<tool>`.
const SERVER: &str = "cryzo";

/// Whether a Claude plan offers this model through Claude Code: the known list, plus newer Opus,
/// Sonnet, Haiku and Fable releases as soon as the catalog has them.
pub fn offered(id: &str) -> bool {
    if MODELS.contains(&id) {
        return true;
    }
    let Some(rest) = id.strip_prefix("claude-") else {
        return false;
    };
    let mut parts = rest.split('-');
    let family = parts.next().unwrap_or_default();
    let version: Vec<&str> = parts.collect();
    ["opus", "sonnet", "haiku", "fable"].contains(&family)
        && !version.is_empty()
        && version.len() <= 2
        && version.iter().all(|part| part.parse::<u32>().is_ok())
        && version[0].parse::<u32>().is_ok_and(|major| major >= 5)
}

/// The user's active "Claude plan" connection, if any.
pub fn connection(engine: &Engine) -> Result<Option<Value>> {
    Ok(engine
        .store
        .desktop_meta("credentials")?
        .into_iter()
        .find(|entry| {
            entry["integrationID"] == "anthropic"
                && entry["active"] == true
                && entry["value"]["methodID"] == METHOD
        }))
}

#[derive(Debug, PartialEq)]
pub enum Status {
    /// `claude` is not installed, or could not be started.
    Missing,
    /// Installed, but not signed in with a Claude account (an API key login doesn't count).
    SignedOut,
    Ready,
}

/// Claude Code's sign-in state, from `claude auth status --json`.
pub async fn status() -> Status {
    match claude(&["auth", "status", "--json"]).await {
        Err(_) => Status::Missing,
        Ok(output) if signed_in(&output) => Status::Ready,
        Ok(_) => Status::SignedOut,
    }
}

/// Signed in with a Claude account, which is what lets a Claude plan pay for the work.
fn signed_in(status: &str) -> bool {
    serde_json::from_str::<Value>(status)
        .is_ok_and(|status| status["loggedIn"] == true && status["authMethod"] == "claude.ai")
}

/// Runs a short `claude` command and returns its standard output.
async fn claude(args: &[&str]) -> Result<String> {
    let mut command = program(args)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(20), command.output())
        .await
        .context("Claude Code did not answer")??;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A command for the user's `claude`: the native installer's copy first, then PATH as a shell
/// finds it (an npm install is `claude.cmd` on Windows). Claude Code signs in with the user's own
/// Claude account, so keys and endpoints that would override that are left out.
fn program(args: &[&str]) -> Result<tokio::process::Command> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(std::path::PathBuf::from)
        .context("home folder not found")?;
    let native = home.join(".local").join("bin").join(if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    });
    let program = if native.is_file() {
        native.to_string_lossy().into_owned()
    } else {
        "claude".into()
    };
    let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    let mut command = crate::tools::command(&program, &args, &home)?;
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
    ] {
        command.env_remove(name);
    }
    Ok(command)
}

// Connecting: the integration's "command" method. Signing in runs Claude Code's own login.

struct Attempt {
    status: &'static str,
    message: String,
    created: u64,
    expires: u64,
    cancel: CancellationToken,
}
static ATTEMPTS: LazyLock<Mutex<HashMap<String, Attempt>>> = LazyLock::new(Default::default);

pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/api/integration/anthropic/connect/command", post(begin))
        .route(
            "/api/integration/anthropic/connect/command/{attempt}",
            get(attempt_status).delete(cancel),
        )
        .with_state(engine)
}

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn error(error: impl ToString) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"_tag":"InvalidRequestError","message":error.to_string()})),
    )
}
fn located(query: &Option<String>, data: Value) -> ApiResult {
    let path = crate::desktop_api::directory(query.as_deref()).map_err(error)?;
    Ok(Json(
        json!({"location":{"directory":path.to_string_lossy().trim_start_matches(r"\\?\")},"data":data}),
    ))
}

async fn begin(
    State(engine): State<Arc<Engine>>,
    RawQuery(query): RawQuery,
    Json(input): Json<Value>,
) -> ApiResult {
    if input["methodID"] != METHOD {
        return Err(error("unknown authentication method"));
    }
    let id = session_store::id("con");
    let created = session_store::now();
    let expires = created + 600_000;
    let cancel = CancellationToken::new();
    let (status, message) = match status().await {
        Status::Missing => {
            return Err(error(
                "Install Claude Code from code.claude.com first, then connect again. Cryzo uses your own Claude Code and never asks for your Claude password.",
            ));
        }
        Status::Ready => {
            save(&engine).map_err(error)?;
            ("complete", String::new())
        }
        Status::SignedOut => {
            tokio::spawn(sign_in(engine.clone(), id.clone(), cancel.clone()));
            (
                "pending",
                "Claude Code is opening your browser to sign in to your Claude account. Cryzo never sees your Claude password or sign-in.".into(),
            )
        }
    };
    let mut attempts = ATTEMPTS.lock().await;
    attempts.retain(|_, attempt| attempt.expires > created);
    attempts.insert(
        id.clone(),
        Attempt {
            status,
            message,
            created,
            expires,
            cancel,
        },
    );
    located(
        &query,
        json!({"attemptID":id,"time":{"created":created,"expires":expires}}),
    )
}

/// Runs `claude auth login` and waits until Claude Code reports a Claude account.
async fn sign_in(engine: Arc<Engine>, attempt: String, cancel: CancellationToken) {
    let result = async {
        let mut login = program(&["auth", "login"])?;
        login
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = login.spawn().context("could not start Claude Code")?;
        // Claude Code prints the sign-in link; showing it covers a browser that didn't open.
        if let Some(stdout) = child.stdout.take() {
            let attempt = attempt.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(entry) = ATTEMPTS.lock().await.get_mut(&attempt)
                        && entry.message.len() < 4000
                    {
                        entry.message.push('\n');
                        entry.message.push_str(line.trim_end());
                    }
                }
            });
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => bail!("Sign-in was cancelled."),
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
            if status().await == Status::Ready {
                save(&engine)?;
                return Ok(());
            }
            ensure!(tokio::time::Instant::now() < deadline, "Sign-in timed out.");
        }
    }
    .await;
    if let Some(entry) = ATTEMPTS.lock().await.get_mut(&attempt) {
        match result {
            Ok(()) => entry.status = "complete",
            Err(error) => {
                entry.status = "failed";
                entry.message = format!("{error:#}");
            }
        }
    }
}

/// Saves the connection. It holds no secret: Claude Code keeps its own sign-in.
fn save(engine: &Engine) -> Result<()> {
    let id = engine
        .store
        .desktop_meta("credentials")?
        .into_iter()
        .find(|entry| entry["integrationID"] == "anthropic")
        .and_then(|entry| entry["id"].as_str().map(str::to_owned))
        .unwrap_or_else(|| session_store::id("cred"));
    let entry = json!({"id":id,"integrationID":"anthropic","label":"Claude plan","active":true,"value":{"type":"external","methodID":METHOD}});
    engine
        .store
        .desktop_put_meta("credentials", &id, Some(&entry))?;
    crate::desktop_models::notify_credentials(&engine.desktop_events, "anthropic", Some(&id));
    Ok(())
}

async fn attempt_status(Path(id): Path<String>, RawQuery(query): RawQuery) -> ApiResult {
    let mut attempts = ATTEMPTS.lock().await;
    let attempt = attempts
        .get_mut(&id)
        .context("sign-in attempt not found")
        .map_err(error)?;
    if attempt.status == "pending" && session_store::now() >= attempt.expires {
        attempt.status = "expired";
        attempt.cancel.cancel();
    }
    located(
        &query,
        json!({"status":attempt.status,"message":attempt.message.trim(),"time":{"created":attempt.created,"expires":attempt.expires}}),
    )
}

async fn cancel(Path(id): Path<String>) -> StatusCode {
    if let Some(attempt) = ATTEMPTS.lock().await.remove(&id) {
        attempt.cancel.cancel();
    }
    StatusCode::NO_CONTENT
}

// Running a turn.

/// What a turn needs from the session engine.
pub(crate) struct Turn<'a> {
    pub session: &'a str,
    pub model: &'a Value,
    pub agent: &'a str,
    pub system: String,
    pub tools: Value,
    pub prompt: Vec<Value>,
    pub workspace: &'a Workspace,
    pub cancel: CancellationToken,
}

/// A tool call from Claude Code, answered with MCP content or an error message.
struct ToolRequest {
    name: String,
    arguments: Value,
    reply: oneshot::Sender<std::result::Result<Value, String>>,
}

/// One Claude Code run: the new prompt in, Claude's steps out as session events, and every tool
/// call executed by Cryzo. Each step ends when the next begins or the run finishes.
pub(crate) async fn run(engine: &Engine, turn: Turn<'_>) -> Result<()> {
    let session = turn.session;
    let model_id = turn.model["id"].as_str().context("model missing")?;
    // One Claude Code conversation per Cryzo session, resumed on later turns.
    let conversation = conversation_id(session);
    let resume = engine
        .store
        .desktop_meta("claude-code")?
        .iter()
        .any(|entry| entry["session"] == session);

    let (calls, mut requests) = mpsc::channel::<ToolRequest>(8);
    let secret = uuid::Uuid::new_v4().simple().to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let bridge = Arc::new(Bridge {
        secret: secret.clone(),
        tools: mcp_tools(&turn.tools),
        calls,
    });
    let stop = CancellationToken::new();
    let server = tokio::spawn({
        let stop = stop.clone();
        let app = Router::new()
            .route("/mcp", post(mcp).get(no_stream).delete(no_stream))
            .with_state(bridge);
        async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await;
        }
    });
    let _server = scopeguard(stop, server);

    // Settings, MCP config and the system prompt go in files, so no argument needs quoting.
    let files = Files::new()?;
    let mcp_config = files.write(
        "mcp.json",
        &json!({"mcpServers":{SERVER:{"type":"http","url":format!("http://{address}/mcp"),"headers":{"Authorization":format!("Bearer {secret}")}}}}).to_string(),
    )?;
    let settings = files.write(
        "settings.json",
        &json!({"disableAllHooks":true,"enabledPlugins":{},"autoMemoryEnabled":false}).to_string(),
    )?;
    let system = files.write("system.md", &turn.system)?;
    let allowed = format!("mcp__{SERVER}");
    let mut args = vec![
        "-p",
        "--restricted",
        "--disable-slash-commands",
        "--no-chrome",
        "--strict-mcp-config",
        "--mcp-config",
        &mcp_config,
        "--settings",
        &settings,
        "--tools",
        "",
        "--permission-mode",
        "dontAsk",
        "--allowedTools",
        &allowed,
        "--system-prompt-file",
        &system,
        "--model",
        model_id,
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
    ];
    args.extend(if resume {
        ["--resume", conversation.as_str()]
    } else {
        ["--session-id", conversation.as_str()]
    });
    let mut command = program(&args)?;
    command
        .current_dir(turn.workspace.root())
        // Approvals can take a while; Claude Code must not give up on a tool call waiting for one.
        .env("MCP_TOOL_TIMEOUT", "86400000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().context(
        "Could not start Claude Code. Install it from code.claude.com, or connect Anthropic with an API key.",
    )?;
    // Ending the turn ends Claude Code and anything it started.
    #[cfg(windows)]
    let mut job = child
        .raw_handle()
        .and_then(|handle| crate::preview::Job::holding(handle).ok());
    if !resume {
        engine.store.desktop_put_meta(
            "claude-code",
            session,
            Some(&json!({"session":session,"conversation":conversation})),
        )?;
    }
    let mut stdin = child.stdin.take().context("Claude Code input")?;
    for message in &turn.prompt {
        let line = json!({"type":"user","message":{"role":"user","content":message}});
        stdin.write_all(format!("{line}\n").as_bytes()).await?;
    }
    // Closing input ends the run once Claude has answered.
    drop(stdin);
    let stderr = child.stderr.take().context("Claude Code errors")?;
    let errors = tokio::spawn(async move {
        let mut text = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if text.len() < 4000 {
                text.push_str(&line);
                text.push('\n');
            }
        }
        text
    });
    let mut lines = BufReader::new(child.stdout.take().context("Claude Code output")?).lines();
    let mut steps = Steps::new(engine, session, turn.model, turn.agent);
    let outcome: Result<()> = loop {
        tokio::select! {
            biased;
            _ = turn.cancel.cancelled() => break Err(anyhow::anyhow!("generation interrupted")),
            request = requests.recv() => {
                let Some(request) = request else { continue };
                let answer = tokio::select! {
                    biased;
                    _ = turn.cancel.cancelled() => break Err(anyhow::anyhow!("generation interrupted")),
                    answer = steps.tool(&request.name, &request.arguments, turn.workspace, &turn.cancel) => answer,
                };
                let _ = request.reply.send(answer);
            }
            line = lines.next_line() => {
                let Some(line) = line? else { break Ok(()) };
                let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(result) = steps.event(&event)? {
                    break result;
                }
            }
        }
    };
    if outcome.is_err() {
        // An npm install runs Claude Code as node under a `claude.cmd` shell, so killing the
        // child alone leaves node running and holding the output pipes. Closing the job ends the
        // whole tree at once.
        #[cfg(windows)]
        drop(job.take());
        let _ = child.start_kill();
    }
    // Nothing may hold the turn open: tool calls still waiting on the bridge get their answer
    // dropped, and a process that ignores the kill is given up on after a few seconds.
    drop(requests);
    drop(lines);
    let grace = std::time::Duration::from_secs(5);
    let status = match tokio::time::timeout(grace, child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            let _ = child.start_kill();
            anyhow::bail!("Claude Code didn't stop in time");
        }
    };
    let errors = tokio::time::timeout(grace, errors)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    steps.finish("stop")?;
    outcome?;
    ensure!(
        status.success() || steps.answered,
        "Claude Code stopped: {}",
        errors.trim().lines().last().unwrap_or("no output")
    );
    Ok(())
}

/// The same Claude Code conversation ID for a session every time, so later turns resume it.
fn conversation_id(session: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("cryzo-claude-code:{session}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string()
}

/// Cryzo's tool definitions (OpenAI function format) as MCP tools.
fn mcp_tools(definitions: &Value) -> Value {
    json!(
        definitions
            .as_array()
            .into_iter()
            .flatten()
            .map(|tool| json!({
                "name": tool["function"]["name"],
                "description": tool["function"]["description"],
                "inputSchema": tool["function"]["parameters"],
            }))
            .collect::<Vec<_>>()
    )
}

/// Follows Claude Code's stream and records it as session steps, the way a provider stream is.
struct Steps<'a> {
    engine: &'a Engine,
    session: &'a str,
    model: &'a Value,
    agent: &'a str,
    /// The open step: its assistant message ID, streamed text and usage.
    step: Option<Step>,
    /// Tool calls Claude has announced, waiting for Cryzo to run them: (call ID, tool name).
    announced: VecDeque<(String, String)>,
    /// Whether Claude produced a final result.
    answered: bool,
}

struct Step {
    message: String,
    parts: crate::native_sessions::StreamParts,
    usage: Value,
    tools: bool,
    streamed: bool,
}

impl<'a> Steps<'a> {
    fn new(engine: &'a Engine, session: &'a str, model: &'a Value, agent: &'a str) -> Self {
        Self {
            engine,
            session,
            model,
            agent,
            step: None,
            announced: VecDeque::new(),
            answered: false,
        }
    }

    /// Handles one stream-json line. `Some` ends the run with that outcome.
    fn event(&mut self, event: &Value) -> Result<Option<Result<()>>> {
        match event["type"].as_str().unwrap_or_default() {
            "stream_event" => self.stream(&event["event"])?,
            "assistant" => {
                if let Some(step) = self.step.as_mut()
                    && event["message"]["usage"].is_object()
                {
                    step.usage = event["message"]["usage"].clone();
                }
            }
            "result" => {
                self.answered = true;
                if event["is_error"] == true {
                    let message = event["result"]
                        .as_str()
                        .filter(|text| !text.is_empty())
                        .or(event["subtype"].as_str())
                        .unwrap_or("Claude Code failed");
                    return Ok(Some(Err(anyhow::anyhow!("{message}"))));
                }
                return Ok(Some(Ok(())));
            }
            _ => {}
        }
        Ok(None)
    }

    fn stream(&mut self, event: &Value) -> Result<()> {
        match event["type"].as_str().unwrap_or_default() {
            "message_start" => {
                // A new model call: the previous step ended in tool calls.
                self.finish("tool-calls")?;
                let message = session_store::id("msg");
                self.engine.record(
                    self.session,
                    "session.step.started",
                    json!({"assistantMessageID":message,"agent":self.agent,"model":self.model,"started":session_store::now()}),
                )?;
                self.step = Some(Step {
                    message,
                    parts: Default::default(),
                    usage: event["message"]["usage"].clone(),
                    tools: false,
                    streamed: false,
                });
            }
            "content_block_start" if event["content_block"]["type"] == "tool_use" => {
                let block = &event["content_block"];
                if let Some(step) = self.step.as_mut() {
                    step.tools = true;
                }
                self.announced.push_back((
                    block["id"].as_str().unwrap_or_default().to_owned(),
                    block["name"].as_str().unwrap_or_default().to_owned(),
                ));
            }
            "content_block_delta" => {
                let delta = &event["delta"];
                let piece = match delta["type"].as_str() {
                    Some("text_delta") => delta["text"]
                        .as_str()
                        .map(|text| crate::provider::Delta::Text(text.into())),
                    Some("thinking_delta") => delta["thinking"]
                        .as_str()
                        .map(|text| crate::provider::Delta::Reasoning(text.into())),
                    _ => None,
                };
                if let (Some(piece), Some(step)) = (piece, self.step.as_mut()) {
                    self.engine.stream_delta(
                        self.session,
                        &step.message,
                        piece,
                        &mut step.parts,
                    )?;
                }
            }
            "message_delta" => {
                if let Some(step) = self.step.as_mut()
                    && let Some(usage) = event["usage"].as_object()
                {
                    for (key, value) in usage {
                        step.usage[key] = value.clone();
                    }
                }
            }
            "message_stop" => self.streamed()?,
            _ => {}
        }
        Ok(())
    }

    /// Closes the text of the open step, once.
    fn streamed(&mut self) -> Result<()> {
        let Some(step) = self.step.as_mut() else {
            return Ok(());
        };
        if step.streamed {
            return Ok(());
        }
        step.streamed = true;
        for (kind, ordinal, text) in [
            (
                "session.text.ended",
                step.parts.text_ordinal,
                &step.parts.text,
            ),
            (
                "session.reasoning.ended",
                step.parts.reasoning_ordinal,
                &step.parts.reasoning,
            ),
        ] {
            if let Some(ordinal) = ordinal {
                self.engine.record(
                    self.session,
                    kind,
                    json!({"assistantMessageID":step.message,"ordinal":ordinal,"text":text,"native":{}}),
                )?;
            }
        }
        self.engine.record(
            self.session,
            "session.step.streamed",
            json!({"assistantMessageID":step.message}),
        )?;
        Ok(())
    }

    /// Ends the open step. A Claude plan is not billed per token, so the cost is zero.
    fn finish(&mut self, finish: &str) -> Result<()> {
        self.streamed()?;
        let Some(step) = self.step.take() else {
            return Ok(());
        };
        let number = |key: &str| step.usage[key].as_u64().unwrap_or(0);
        let finish = if step.tools { "tool-calls" } else { finish };
        self.engine.record(
            self.session,
            "session.step.ended",
            json!({"assistantMessageID":step.message,"finish":finish,"cost":0,"tokens":{
                "input":number("input_tokens"),"output":number("output_tokens"),"reasoning":0,
                "cache":{"read":number("cache_read_input_tokens"),"write":number("cache_creation_input_tokens")}}}),
        )?;
        Ok(())
    }

    /// Runs one of Cryzo's tools for Claude, recorded on the open step like any model's tool call.
    async fn tool(
        &mut self,
        name: &str,
        arguments: &Value,
        workspace: &Workspace,
        cancel: &CancellationToken,
    ) -> std::result::Result<Value, String> {
        let qualified = format!("mcp__{SERVER}__{name}");
        let id = self
            .announced
            .iter()
            .position(|(_, announced)| *announced == qualified || announced == name)
            .and_then(|index| self.announced.remove(index))
            .map(|(id, _)| id)
            .unwrap_or_else(|| session_store::id("call"));
        // The text before a tool call belongs to its step, so close it before the call shows.
        self.streamed().map_err(|error| error.to_string())?;
        let message = match self.step.as_ref() {
            Some(step) => step.message.clone(),
            None => return Err("no step is running".into()),
        };
        let plan = self.agent == "plan";
        let answer = self
            .engine
            .call_tool(
                self.session,
                &message,
                &id,
                name,
                &arguments.to_string(),
                plan,
                workspace,
                cancel,
                &json!({}),
            )
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
        // Claude Code's input is closed for the whole run, so messages the user sends meanwhile
        // reach Claude with the next tool result: each one is a step boundary, where Cryzo's own
        // loop reads its inbox too.
        let steers = self.steers().unwrap_or_default();
        if steers.is_empty() {
            return answer;
        }
        let note = format!(
            "The user sent this while you were working. Read it now and follow it, even if it changes or stops the task:\n{}",
            steers.join("\n\n")
        );
        match answer {
            Ok(Value::Array(mut parts)) => {
                parts.push(json!({"type":"text","text":note}));
                Ok(Value::Array(parts))
            }
            Ok(other) => {
                Ok(json!([{"type":"text","text":other.to_string()},{"type":"text","text":note}]))
            }
            Err(error) => Err(format!("{error}\n\n{note}")),
        }
    }

    /// Delivers the user's queued messages into the conversation and returns their text.
    fn steers(&self) -> Result<Vec<String>> {
        let pending = self.engine.store.desktop_state(self.session)?["inbox"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut texts = Vec::new();
        for item in pending.iter().filter(|item| item["type"] == "user") {
            self.engine.record(
                self.session,
                "session.inbox.delivered",
                json!({"inboxID":item["id"]}),
            )?;
            if let Some(text) = item["payload"]["text"].as_str() {
                texts.push(text.to_owned());
            }
        }
        Ok(texts)
    }
}

/// The local MCP server Claude Code calls Cryzo's tools through, for one run.
struct Bridge {
    secret: String,
    tools: Value,
    calls: mpsc::Sender<ToolRequest>,
}

/// Streamable HTTP MCP, answering each request with plain JSON (no server-sent stream).
async fn mcp(State(bridge): State<Arc<Bridge>>, headers: HeaderMap, body: String) -> Response {
    let authorized = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == format!("Bearer {}", bridge.secret));
    if !authorized {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(request) = serde_json::from_str::<Value>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    // Notifications carry no ID and get no answer.
    let Some(id) = request.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let result = match request["method"].as_str().unwrap_or_default() {
        "initialize" => Ok(json!({
            "protocolVersion": request["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": SERVER, "version": env!("CARGO_PKG_VERSION")},
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": bridge.tools})),
        "tools/call" => {
            let (reply, answer) = oneshot::channel();
            let request = ToolRequest {
                name: request["params"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                arguments: request["params"]["arguments"].clone(),
                reply,
            };
            Ok(match bridge.calls.send(request).await {
                Err(_) => tool_error("The Cryzo session ended."),
                Ok(()) => match answer.await {
                    Ok(Ok(content)) => json!({"content": mcp_content(&content)}),
                    Ok(Err(message)) => tool_error(&message),
                    Err(_) => tool_error("The Cryzo session ended."),
                },
            })
        }
        _ => Err(json!({"code": -32601, "message": "method not found"})),
    };
    let body = match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(error) => json!({"jsonrpc":"2.0","id":id,"error":error}),
    };
    Json(body).into_response()
}

async fn no_stream() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

fn tool_error(message: &str) -> Value {
    json!({"content":[{"type":"text","text":message}],"isError":true})
}

/// Cryzo's tool content parts (text, and image files as data URIs) as MCP content.
fn mcp_content(content: &Value) -> Value {
    json!(
        content
            .as_array()
            .into_iter()
            .flatten()
            .map(|part| {
                let image = part["type"] == "file"
                    && part["mime"].as_str().is_some_and(|mime| mime.starts_with("image/"));
                let data = part["uri"]
                    .as_str()
                    .and_then(|url| url.split_once(";base64,"))
                    .map(|(_, data)| data);
                match (image, data) {
                    (true, Some(data)) => json!({"type":"image","data":data,"mimeType":part["mime"]}),
                    _ => json!({"type":"text","text":part["text"].as_str().map_or_else(|| part.to_string(), str::to_owned)}),
                }
            })
            .collect::<Vec<_>>()
    )
}

/// Stops the MCP server when the run ends, however it ends.
fn scopeguard(stop: CancellationToken, server: tokio::task::JoinHandle<()>) -> impl Drop {
    struct Guard(CancellationToken, tokio::task::JoinHandle<()>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.cancel();
            self.1.abort();
        }
    }
    Guard(stop, server)
}

/// The run's settings, MCP config and system prompt, removed when the run ends.
struct Files(std::path::PathBuf);

impl Files {
    fn new() -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("cryzo-claude-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&directory)?;
        Ok(Self(directory))
    }

    fn write(&self, name: &str, contents: &str) -> Result<String> {
        let path = self.0.join(name);
        std::fs::write(&path, contents)?;
        Ok(path.to_string_lossy().into_owned())
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_sent_during_a_run_are_delivered_with_the_next_tool_result() {
        let folder = tempfile::tempdir().unwrap();
        let engine = Engine::open(&folder.path().join("cryzo.sqlite")).unwrap();
        let info = json!({"id":"ses_steer","title":"t","agent":"build","location":{"directory":folder.path()},"time":{"created":1,"updated":1},"cost":0,"tokens":session_store::tokens()});
        engine.store.desktop_create(info).unwrap();
        let model = json!({"id":"claude-opus-5-5","providerID":"anthropic"});
        let steps = Steps::new(&engine, "ses_steer", &model, "build");
        assert!(steps.steers().unwrap().is_empty());
        engine
            .record(
                "ses_steer",
                "session.inbox.enqueued",
                json!({"inboxID":"msg_steer","item":{"type":"user","payload":{"text":"Stop and use blue instead"},"delivery":"steer"}}),
            )
            .unwrap();
        assert_eq!(steps.steers().unwrap(), ["Stop and use blue instead"]);
        let state = engine.store.desktop_state("ses_steer").unwrap();
        assert!(
            state["inbox"].as_array().unwrap().is_empty(),
            "delivered, not left pending"
        );
        assert_eq!(state["messages"][0]["text"], "Stop and use blue instead");
        assert!(
            steps.steers().unwrap().is_empty(),
            "each message is delivered once"
        );
    }

    #[test]
    fn new_claude_and_gpt_releases_are_offered_on_the_plans() {
        assert!(offered("claude-opus-5-5"));
        assert!(
            offered("claude-sonnet-6"),
            "a newer release shows up without an update"
        );
        assert!(offered("claude-haiku-6-1"));
        assert!(!offered("claude-opus-4-8"), "older than the plan's models");
        assert!(!offered("claude-sonnet-5-5-20260101"));
        assert!(crate::chatgpt::offered("gpt-6.2-sol"));
        assert!(crate::chatgpt::offered("gpt-6.2-astra"));
        assert!(crate::chatgpt::offered("gpt-7-terra"));
        assert!(crate::chatgpt::offered("gpt-5.6-luna"), "on the known list");
        assert!(!crate::chatgpt::offered("gpt-6-cyber"), "not a plan family");
        assert!(!crate::chatgpt::offered("gpt-5.4-sol"));
    }

    #[test]
    fn the_built_in_catalog_passes_the_check_a_downloaded_one_must_pass() {
        let catalog = crate::catalog::Catalog::load().unwrap();
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("models.json"), "{not json").unwrap();
        let saved = crate::catalog::Catalog::load_saved(folder.path()).unwrap();
        assert_eq!(
            saved.providers.len(),
            catalog.providers.len(),
            "a broken saved copy falls back"
        );
    }

    #[test]
    fn only_a_claude_account_counts_as_signed_in() {
        assert!(signed_in(
            r#"{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"pro"}"#
        ));
        assert!(!signed_in(r#"{"loggedIn":true,"authMethod":"apiKey"}"#));
        assert!(!signed_in(r#"{"loggedIn":false,"authMethod":"claude.ai"}"#));
        assert!(!signed_in("not json"));
    }

    #[test]
    fn a_session_keeps_one_claude_conversation() {
        let first = conversation_id("ses_1");
        assert_eq!(first, conversation_id("ses_1"));
        assert_ne!(first, conversation_id("ses_2"));
        assert!(uuid::Uuid::parse_str(&first).is_ok());
    }

    #[test]
    fn tools_and_content_translate_to_mcp() {
        let tools = mcp_tools(
            &json!([{"type":"function","function":{"name":"read","description":"Read a file","parameters":{"type":"object"}}}]),
        );
        assert_eq!(
            tools,
            json!([{"name":"read","description":"Read a file","inputSchema":{"type":"object"}}])
        );
        let content = mcp_content(&json!([
            {"type":"text","text":"hello"},
            {"type":"file","mime":"image/jpeg","uri":"data:image/jpeg;base64,QUJD"}
        ]));
        assert_eq!(
            content,
            json!([{"type":"text","text":"hello"},{"type":"image","data":"QUJD","mimeType":"image/jpeg"}])
        );
    }
}
