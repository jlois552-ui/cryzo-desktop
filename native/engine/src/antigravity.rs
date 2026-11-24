//! Preview transport for Google's personal-account ACP server. The editor never handles a Google token.
use crate::{
    native_sessions::{StreamParts, external_prompt},
    service::Engine,
    session_store,
};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    extract::{Path as Param, RawQuery, State},
    routing::{get, post},
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
};
use tokio_util::{
    codec::{FramedRead, LinesCodec},
    sync::CancellationToken,
};

pub const ID: &str = "antigravity";
const METHOD: &str = "antigravity-personal";
pub fn preview() -> bool {
    std::env::var("CRYZO_ANTIGRAVITY_ACP").as_deref() == Ok("1")
}
fn executable() -> Result<PathBuf> {
    let name = if cfg!(windows) { "agy.exe" } else { "agy" };
    let path = std::env::var_os("CRYZO_AGY_PATH")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .filter(|p| p.is_absolute())
                .map(|p| p.join(name))
                .find(|p| p.is_file())
        })
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("agy/bin").join(name))
        })
        .filter(|p| p.is_absolute() && p.is_file())
        .context("Install Antigravity CLI and run agy acp --login.")?;
    Ok(path)
}

struct Client<I, O> {
    input: I,
    output: FramedRead<BufReader<O>, LinesCodec>,
    next: u64,
}
impl Client<ChildStdin, ChildStdout> {
    fn open(cwd: &Path) -> Result<(Child, Self)> {
        let mut command = Command::new(executable()?);
        command
            .args(["acp", "--sandbox", "--include-default-customizations=false"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for key in [
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GOOGLE_GENAI_USE_VERTEXAI",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "GOOGLE_CLOUD_PROJECT",
            "GOOGLE_CLOUD_LOCATION",
        ] {
            command.env_remove(key);
        }
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn()?;
        let input = child.stdin.take().context("missing ACP stdin")?;
        let output = child.stdout.take().context("missing ACP stdout")?;
        Ok((child, Self::new(input, output)))
    }
}

impl<I: AsyncWrite + Unpin, O: AsyncRead + Unpin> Client<I, O> {
    fn new(input: I, output: O) -> Self {
        Self {
            input,
            output: FramedRead::new(
                BufReader::new(output),
                LinesCodec::new_with_max_length(2 * 1024 * 1024),
            ),
            next: 0,
        }
    }
    async fn send(&mut self, mut value: Value) -> Result<()> {
        value["jsonrpc"] = json!("2.0");
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        self.input.write_all(&bytes).await?;
        self.input.flush().await?;
        Ok(())
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<u64> {
        self.next += 1;
        let id = self.next;
        self.send(json!({"id":id,"method":method,"params":params}))
            .await?;
        Ok(id)
    }
    async fn read(&mut self) -> Result<Value> {
        let line = self
            .output
            .next()
            .await
            .context("Antigravity CLI disconnected")??;
        Ok(serde_json::from_str(&line)?)
    }
    async fn reject(&mut self, v: &Value) -> Result<()> {
        if v["method"] == "session/request_permission" {
            self.send(json!({"id":v["id"],"result":{"outcome":{"outcome":"cancelled"}}}))
                .await
        } else {
            self.send(
                json!({"id":v["id"],"error":{"code":-32601,"message":"Unsupported client method"}}),
            )
            .await
        }
    }
    async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.request(method, params).await?;
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let v = self.read().await?;
                if v.get("method").is_some() {
                    if !v["id"].is_null() {
                        self.reject(&v).await?;
                    }
                } else if v["id"] == id {
                    return response(v);
                }
            }
        })
        .await
        .context("Antigravity CLI timed out")?
    }
    async fn init(&mut self) -> Result<Value> {
        let v=self.call("initialize",json!({"protocolVersion":1,"clientInfo":{"name":"cryzo","version":env!("CARGO_PKG_VERSION")},"clientCapabilities":{}})).await?;
        ensure!(v["protocolVersion"] == 1, "Unsupported ACP version");
        ensure!(
            v["authMethods"]
                .as_array()
                .is_some_and(|m| m.iter().any(|a| a["id"] == "oauth-personal")),
            "Personal Google sign-in unavailable"
        );
        Ok(v)
    }
}
fn response(mut v: Value) -> Result<Value> {
    if v["error"]["code"] == -32000 {
        bail!("Sign in with agy acp --login, then connect again.");
    }
    if let Some(err) = v.get("error") {
        bail!(
            "Antigravity: {}",
            err["message"].as_str().unwrap_or("request failed")
        );
    }
    v.as_object_mut()
        .and_then(|o| o.remove("result"))
        .context("Invalid ACP response")
}
fn selector<'a>(state: &'a Value, category: &str) -> Option<&'a Value> {
    state["configOptions"]
        .as_array()?
        .iter()
        .find(|option| option["category"] == category || option["id"] == category)
}

fn choices(option: &Value) -> impl Iterator<Item = &Value> {
    option["options"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|choice| {
            choice["options"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(std::slice::from_ref(choice))
        })
}

fn options(v: &Value) -> Result<(&str, Vec<Value>)> {
    let config = selector(v, "model").context("Antigravity did not advertise a model picker")?;
    let models: Vec<_> = choices(config).filter_map(|choice| {
        let id = choice["value"].as_str()?;
        let name = choice["name"].as_str()?;
        Some(json!({"id":id,"modelID":id,"providerID":ID,"name":name,"type":"chat","family":id,
            "capabilities":{"tools":true,"input":["text"],"output":["text"]},"variants":[],
            "time":{"released":0},"cost":[],"status":"active","enabled":true,"limit":{"context":0,"output":0}}))
    }).collect();
    ensure!(
        !models.is_empty(),
        "Antigravity did not advertise any models"
    );
    Ok((
        config["id"].as_str().context("Missing model config ID")?,
        models,
    ))
}
pub fn models(engine: &Engine) -> Result<Vec<Value>> {
    if !preview() {
        return Ok(vec![]);
    }
    Ok(engine
        .store
        .desktop_meta("credentials")?
        .into_iter()
        .find(|v| v["integrationID"] == ID && v["active"] == true)
        .and_then(|v| {
            v.pointer("/value/metadata/models")
                .and_then(Value::as_array)
                .cloned()
        })
        .unwrap_or_default())
}
pub fn selected(engine: &Engine, model: &Value) -> Result<()> {
    ensure!(
        model["variant"].is_null() && models(engine)?.iter().any(|m| m["id"] == model["id"]),
        "Reconnect Antigravity to select an advertised model"
    );
    Ok(())
}
pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route(
            "/api/integration/antigravity/connect/command",
            post(connect),
        )
        .route(
            "/api/integration/antigravity/connect/command/{attempt}",
            get(status),
        )
        .with_state(engine)
}
type ApiResult = Result<Json<Value>, (axum::http::StatusCode, Json<Value>)>;
fn error(e: impl ToString) -> (axum::http::StatusCode, Json<Value>) {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(json!({"_tag":"InvalidRequestError","message":e.to_string()})),
    )
}
fn located(q: Option<String>, data: Value) -> ApiResult {
    let dir = crate::desktop_api::directory(q.as_deref()).map_err(error)?;
    Ok(Json(
        json!({"location":{"directory":dir.to_string_lossy().trim_start_matches(r"\\?\")},"data":data}),
    ))
}
async fn connect(
    State(engine): State<Arc<Engine>>,
    RawQuery(q): RawQuery,
    Json(input): Json<Value>,
) -> ApiResult {
    if !preview() || input["methodID"] != METHOD {
        return Err(error("Antigravity preview is disabled"));
    }
    let scratch = tempfile::tempdir().map_err(error)?;
    let (_child, mut client) = Client::open(scratch.path()).map_err(error)?;
    client.init().await.map_err(error)?;
    let state = client
        .call("session/new", json!({"cwd":scratch.path(),"mcpServers":[]}))
        .await
        .map_err(error)?;
    let (_, models) = options(&state).map_err(error)?;
    check_local_rules().map_err(error)?;
    let id = "cred_antigravity";
    let created = session_store::now();
    let attempt = session_store::id("con");
    let value = json!({"id":id,"integrationID":ID,"label":"Antigravity CLI","active":true,"value":{"type":"external","methodID":METHOD,"metadata":{"models":models,"attempt":attempt,"created":created}}});
    engine
        .store
        .desktop_put_meta("credentials", id, Some(&value))
        .map_err(error)?;
    crate::desktop_models::notify_credentials(&engine.desktop_events, ID, Some(id));
    located(
        q,
        json!({"attemptID":attempt,"time":{"created":created,"expires":created+600000}}),
    )
}
fn check_local_rules() -> Result<()> {
    let home = std::env::home_dir().context("Home directory unavailable")?;
    let settings = std::fs::read(home.join(".gemini/antigravity-cli/settings.json"))
        .context("Configure Antigravity CLI permission rules before running a project")?;
    check_rules(&serde_json::from_slice(&settings)?)
}

async fn status(
    State(engine): State<Arc<Engine>>,
    Param(id): Param<String>,
    RawQuery(q): RawQuery,
) -> ApiResult {
    let row = engine
        .store
        .desktop_meta("credentials")
        .map_err(error)?
        .into_iter()
        .find(|v| v["integrationID"] == ID && v["value"]["metadata"]["attempt"] == id)
        .ok_or_else(|| error("Unknown connection attempt"))?;
    let created = row["value"]["metadata"]["created"]
        .as_i64()
        .unwrap_or_default();
    located(
        q,
        json!({"status":"complete","message":"","time":{"created":created,"expires":created+600000}}),
    )
}

// The CLI implicitly allows project writes. Require explicit Ask (or Deny) rules
// before giving a signed-in agent a project, including after a settings change.
fn check_rules(settings: &Value) -> Result<()> {
    for action in [
        "write_file(*)",
        "command(*)",
        "mcp(*)",
        "execute_url(*)",
        "unsandboxed(*)",
    ] {
        ensure!(
            ["ask", "deny"]
                .iter()
                .any(|list| settings["permissions"][list]
                    .as_array()
                    .is_some_and(|rules| rules.iter().any(|r| r == action))),
            "Antigravity settings must include {action} in permissions.ask or permissions.deny for exact Cryzo approvals"
        );
    }
    Ok(())
}
pub async fn run(
    engine: &Engine,
    session: &str,
    model: &Value,
    agent: &str,
    cwd: &Path,
    cancel: CancellationToken,
) -> Result<()> {
    selected(engine, model)?;
    check_local_rules()?;
    let (mut child, mut client) = Client::open(cwd)?;
    let turn = async {
        let init = interruptible(&cancel, client.init()).await?;
        let checkpoint = engine
            .store
            .desktop_meta("antigravity-session")?
            .into_iter()
            .find(|v| v["session"] == session);
        let old = checkpoint.as_ref().filter(|previous| {
            previous["agent"] == agent
                && init
                    .pointer("/agentCapabilities/sessionCapabilities/resume")
                    .is_some_and(Value::is_object)
        });
        let args = json!({"cwd":cwd.to_string_lossy().trim_start_matches(r"\\?\"),"mcpServers":[]});
        let config = if let Some(old) = old {
            let mut args = args;
            args["sessionId"] = old["remote"].clone();
            interruptible(&cancel, client.call("session/resume", args)).await?
        } else {
            interruptible(&cancel, client.call("session/new", args)).await?
        };
        let remote = old
            .map(|v| &v["remote"])
            .unwrap_or(&config["sessionId"])
            .as_str()
            .context("Missing ACP session ID")?;
        let (key, advertised) = options(&config)?;
        ensure!(
            advertised.iter().any(|m| m["id"] == model["id"]),
            "Model is no longer available to this Antigravity account"
        );
        interruptible(
            &cancel,
            client.call(
                "session/set_config_option",
                json!({"sessionId":remote,"configId":key,"value":model["id"]}),
            ),
        )
        .await?;
        if agent == "plan" {
            let is_plan = |id: &Value| id == "plan" || id == "planning";
            if let Some(option) = selector(&config, "mode")
                && let Some(mode) = choices(option).find(|item| is_plan(&item["value"]))
            {
                interruptible(
                    &cancel,
                    client.call(
                        "session/set_config_option",
                        json!({"sessionId":remote,"configId":option["id"],"value":mode["value"]}),
                    ),
                )
                .await?;
            } else {
                let mode = config["modes"]["availableModes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|value| is_plan(&value["id"]))
                    .context("Antigravity did not advertise a read-only plan mode")?;
                interruptible(
                    &cancel,
                    client.call(
                        "session/set_mode",
                        json!({"sessionId":remote,"modeId":mode["id"]}),
                    ),
                )
                .await?;
            }
        }
        let state = engine.store.desktop_state(session)?;
        for item in state["inbox"].as_array().into_iter().flatten() {
            engine.record(
                session,
                "session.inbox.delivered",
                json!({"inboxID":item["id"]}),
            )?;
        }
        let prompt = external_prompt(
            &engine.store.desktop_state(session)?,
            old.and_then(|v| v["lastMessage"].as_str()),
            24000,
        );
        let mut blocks = vec![
            json!({"type":"text","text":if agent=="plan" {"Cryzo plan mode. Inspect and plan only."}else{"Continue the user's active Cryzo task. Respect project instructions and exact approvals."}}),
        ];
        for block in prompt.iter().filter_map(Value::as_array).flatten() {
            if block["type"] == "text" {
                blocks.push(block.clone());
            } else {
                bail!("This Antigravity preview supports text prompts only");
            }
        }
        let request = client
            .request(
                "session/prompt",
                json!({"sessionId":remote,"prompt":blocks}),
            )
            .await?;
        let message = session_store::id("msg");
        engine.record(
            session,
            "session.step.started",
            json!({"assistantMessageID":message,"agent":agent,"model":model}),
        )?;
        let mut parts = StreamParts::default();
        let save_checkpoint =
            || {
                engine.store.desktop_put_meta("antigravity-session", session,
            Some(&json!({"session":session,"remote":remote,"lastMessage":message,"agent":agent})))
            };
        let mut acknowledged = false;
        let result = async {
            loop {
                let v = interruptible(&cancel, client.read()).await?;
                if v.get("method").is_none() && v["id"] == request {
                    let result = response(v)?;
                    ensure!(
                        result["stopReason"] == "end_turn",
                        "Antigravity stopped: {}",
                        result["stopReason"]
                    );
                    break;
                }
                if !v["id"].is_null()
                    && v["method"] == "session/request_permission"
                    && v["params"]["sessionId"] == remote
                {
                    let params = &v["params"];
                    let call = &params["toolCall"];
                    let operation = json!({"tool":"antigravity","request":call});
                    let once = allow_once(params, agent);
                    let mut outcome = json!({"outcome":"cancelled"});
                    if let (Some(once), Some(call_id)) = (once, call["toolCallId"].as_str())
                        && let Ok(approval) = engine
                            .ask_operation(session, &message, call_id, &operation, cancel.clone())
                            .await
                    {
                        engine
                            .store
                            .consume_approval(session, &approval, &operation)?;
                        outcome = json!({"outcome":"selected","optionId":once});
                    }
                    client
                        .send(json!({"id":v["id"],"result":{"outcome":outcome}}))
                        .await?;
                } else if !v["id"].is_null() && v.get("method").is_some() {
                    client.reject(&v).await?;
                } else if v["method"] == "session/update" && v["params"]["sessionId"] == remote {
                    let update = &v["params"]["update"];
                    if !acknowledged
                        && matches!(
                            update["sessionUpdate"].as_str(),
                            Some(
                                "agent_message_chunk"
                                    | "agent_thought_chunk"
                                    | "tool_call"
                                    | "tool_call_update"
                            )
                        )
                    {
                        save_checkpoint()?;
                        acknowledged = true;
                    }
                    if update["sessionUpdate"] == "agent_message_chunk"
                        && let Some(text) = update["content"]["text"].as_str()
                    {
                        engine.stream_delta(
                            session,
                            &message,
                            crate::provider::Delta::Text(text.into()),
                            &mut parts,
                        )?;
                    } else if matches!(
                        update["sessionUpdate"].as_str(),
                        Some("tool_call" | "tool_call_update")
                    ) {
                        let result = update
                            .get("rawOutput")
                            .filter(|value| !value.is_null())
                            .map(|value| value.to_string().chars().take(900).collect::<String>())
                            .unwrap_or_default();
                        let detail = format!(
                            "\n[Antigravity tool {}: {} {}]\n{result}",
                            update["toolCallId"].as_str().unwrap_or(""),
                            update["title"].as_str().unwrap_or(""),
                            update["status"].as_str().unwrap_or("")
                        );
                        engine.stream_delta(
                            session,
                            &message,
                            crate::provider::Delta::Text(detail),
                            &mut parts,
                        )?;
                    }
                }
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if cancel.is_cancelled() {
            let _ = tokio::time::timeout(
                Duration::from_millis(500),
                client.send(json!({"method":"session/cancel","params":{"sessionId":remote}})),
            )
            .await;
        }
        if let Some(ordinal) = parts.text_ordinal {
            engine.record(
                session,
                "session.text.ended",
                json!({"assistantMessageID":message,"ordinal":ordinal,"text":parts.text}),
            )?;
        }
        if let Err(err) = result {
            engine.record(session,"session.step.failed",json!({"assistantMessageID":message,"error":{"type":"NativeError","message":err.to_string()}}))?;
            return Err(err);
        }
        engine.record(session,"session.step.ended",json!({"assistantMessageID":message,"finish":"stop","tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},"cost":0}))?;
        if !acknowledged {
            save_checkpoint()?;
        }
        Ok(())
    };
    let result = turn.await;
    if cancel.is_cancelled() {
        // Closing the CLI process also stops the active ACP session.
        let _ = child.kill().await;
    }
    result
}

async fn interruptible<T>(
    cancel: &CancellationToken,
    action: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => bail!("Antigravity interrupted"),
        result = action => result,
    }
}

fn allow_once<'a>(params: &'a Value, agent: &str) -> Option<&'a str> {
    if agent == "plan" || !params["toolCall"]["rawInput"].is_object() {
        return None;
    }
    params["options"]
        .as_array()?
        .iter()
        .find(|option| option["kind"] == "allow_once")?["optionId"]
        .as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acp_negotiates_and_discovers_the_agents_models() {
        let (editor, agent) = tokio::io::duplex(8192);
        let (read, write) = tokio::io::split(editor);
        let mut client = Client::new(write, read);
        let mock = tokio::spawn(async move {
            let (read, write) = tokio::io::split(agent);
            let mut agent = Client::new(write, read);
            let init = agent.read().await.unwrap();
            assert_eq!(init["method"], "initialize");
            assert_eq!(init["params"]["protocolVersion"], 1);
            agent.send(json!({"id": init["id"],"result":{"protocolVersion":1,"authMethods":[{"id":"oauth-personal"}]}})).await.unwrap();
            let create = agent.read().await.unwrap();
            assert_eq!(create["method"], "session/new");
            // A server request during setup may never be silently approved.
            agent
                .send(json!({"id":"approval","method":"session/request_permission","params":{}}))
                .await
                .unwrap();
            let rejected = agent.read().await.unwrap();
            assert_eq!(rejected["id"], "approval");
            assert_eq!(
                rejected.pointer("/result/outcome/outcome").unwrap(),
                "cancelled"
            );
            agent.send(json!({"id":create["id"],"result":{"sessionId":"fixture","configOptions":[
                {"id":"model-id","category":"model","options":[{"group":"Available","options":[{"value":"example-model","name":"Example Model"}]}]}
            ]}})).await.unwrap();
        });
        client.init().await.unwrap();
        let state = client
            .call("session/new", json!({"cwd":"/fixture","mcpServers":[]}))
            .await
            .unwrap();
        let (key, available) = options(&state).unwrap();
        assert_eq!(key, "model-id");
        assert_eq!(available[0]["id"], "example-model");
        assert_eq!(available.len(), 1);
        mock.await.unwrap();
    }

    #[test]
    fn approval_requires_one_exact_operation() {
        let mut params = json!({"toolCall":{"rawInput":{"path":"note.txt"}},"options":[
            {"kind":"allow_always","optionId":"always"},{"kind":"allow_once","optionId":"once"}
        ]});
        assert_eq!(allow_once(&params, "build"), Some("once"));
        assert_eq!(allow_once(&params, "plan"), None);
        params["toolCall"]["rawInput"] = Value::Null;
        assert_eq!(allow_once(&params, "build"), None);
        assert!(check_rules(&json!({})).is_err());
    }
}
