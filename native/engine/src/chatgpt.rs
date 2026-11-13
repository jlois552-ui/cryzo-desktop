//! Sign in with ChatGPT through OpenAI's token sharing for agents, ported from the upstream
//! `chatgpt.ts` provider plugin. First sign-in registers a client owned by the user; requests then
//! use their ChatGPT plan through api.openai.com.
use crate::{credentials::Credentials, service::Engine, session_store};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    extract::{Path, Query, RawQuery, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, Notify};

pub const METHOD: &str = "chatgpt-token-sharing";
const ISSUER: &str = "https://auth.openai.com";
const RESOURCE: &str = "https://api.openai.com/v1";
const SCOPE: &str = "chatgpt.tokens.use.direct";
/// Models ChatGPT plans serve through token sharing (upstream's list, where the catalog has them).
pub const MODELS: &[&str] = &[
    "gpt-5.5",
    "gpt-5.6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-6-astra",
    "gpt-6-luna",
    "gpt-6-sol",
    "gpt-6.1-sol",
    "gpt-6.1-terra",
];

pub struct Attempt {
    status: String,
    message: Option<String>,
    created: u64,
    expires: u64,
    done: Arc<Notify>,
}
pub type Attempts = Mutex<HashMap<String, Attempt>>;
/// Access tokens stay in memory. Only the refresh token is saved, in the OS credential store,
/// which also keeps it within Windows Credential Manager's size limit.
pub type Token = Mutex<Option<(String, u64)>>;

struct Flow {
    engine: Arc<Engine>,
    attempt: String,
    state: String,
    nonce: String,
    verifier: String,
    redirect: String,
    saved: Option<String>,
    done: Arc<Notify>,
}

pub fn connection(engine: &Engine) -> Result<Option<Value>> {
    Ok(engine
        .store
        .desktop_meta("credentials")?
        .into_iter()
        .find(|entry| {
            entry["integrationID"] == "openai"
                && entry["active"] == true
                && entry["value"]["methodID"] == METHOD
        }))
}

/// Refreshes before expiry. The lock also serializes refreshes, since each one rotates the refresh token.
pub async fn access_token(engine: &Engine) -> Result<String> {
    let mut cached = engine.chatgpt_token.lock().await;
    if let Some((token, expires)) = cached.as_ref()
        && *expires > session_store::now() + 60_000
    {
        return Ok(token.clone());
    }
    let client_id = connection(engine)?
        .and_then(|entry| {
            entry
                .pointer("/value/metadata/clientID")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .context("reconnect ChatGPT")?;
    let refresh = tokio::task::spawn_blocking(|| Credentials::get("openai")).await??;
    let tokens = token_request(&[
        ("grant_type", "refresh_token"),
        ("client_id", &client_id),
        ("refresh_token", &refresh),
        ("resource", RESOURCE),
    ])
    .await
    .context("ChatGPT sign-in expired; reconnect ChatGPT")?;
    keep(&tokens, &mut cached).await
}

pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/api/integration/openai/connect/oauth", post(begin))
        .route(
            "/api/integration/openai/connect/oauth/{attempt}",
            get(status).delete(cancel),
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
    let host = host(&engine).map_err(error)?;
    // Reauthorization reuses the client this user registered on first sign-in.
    let saved = connection(&engine).map_err(error)?.and_then(|entry| {
        entry
            .pointer("/value/metadata/clientID")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    const VERIFIER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let verifier: String = random(43)
        .iter()
        .map(|byte| VERIFIER[*byte as usize % VERIFIER.len()] as char)
        .collect();
    let state = URL_SAFE_NO_PAD.encode(random(32));
    let nonce = URL_SAFE_NO_PAD.encode(random(32));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(error)?;
    let redirect = format!(
        "http://127.0.0.1:{}/auth/callback",
        listener.local_addr().map_err(error)?.port()
    );
    let mut url = url::Url::parse(&format!("{ISSUER}/api/accounts/authorize")).map_err(error)?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair(
            "client_id",
            saved.as_deref().unwrap_or("dynamic_agent_client"),
        );
        if saved.is_none() {
            pairs.append_pair("agent_name_hint", "Cryzo");
        }
        pairs
            .append_pair("ext_agent_host_id", &host)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect)
            .append_pair(
                "scope",
                &format!("openid profile email offline_access resource.invoke {SCOPE}"),
            )
            .append_pair("resource", RESOURCE)
            .append_pair("state", &state)
            .append_pair("nonce", &nonce)
            .append_pair("code_challenge_method", "S256")
            .append_pair(
                "code_challenge",
                &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            );
    }
    let id = session_store::id("con");
    let created = session_store::now();
    let expires = created + 600_000;
    let done = Arc::new(Notify::new());
    {
        let mut attempts = engine.chatgpt_attempts.lock().await;
        attempts.retain(|_, attempt| attempt.expires > created);
        if attempts.len() >= 8 {
            return Err(error("too many pending sign-ins"));
        }
        attempts.insert(
            id.clone(),
            Attempt {
                status: "pending".into(),
                message: None,
                created,
                expires,
                done: done.clone(),
            },
        );
    }
    let flow = Arc::new(Flow {
        engine,
        attempt: id.clone(),
        state,
        nonce,
        verifier,
        redirect,
        saved,
        done: done.clone(),
    });
    let callback = Router::new()
        .route("/auth/callback", get(callback))
        .with_state(flow);
    tokio::spawn(async move {
        let _ = axum::serve(listener, callback)
            .with_graceful_shutdown(async move {
                tokio::select! {
                    _ = done.notified() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_secs(600)) => {}
                }
            })
            .await;
    });
    located(
        &query,
        // Empty instructions: the dialog would otherwise show them as a confirmation code, which this flow lacks.
        json!({"attemptID":id,"url":url.to_string(),"instructions":"","mode":"auto","time":{"created":created,"expires":expires}}),
    )
}

async fn callback(
    State(flow): State<Arc<Flow>>,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Html<String>) {
    let pending = flow
        .engine
        .chatgpt_attempts
        .lock()
        .await
        .get(&flow.attempt)
        .is_some_and(|attempt| attempt.status == "pending");
    if !pending {
        return (
            StatusCode::CONFLICT,
            Html(page(Some("This sign-in already finished."))),
        );
    }
    let message = complete(&flow, &params)
        .await
        .err()
        .map(|error| error.to_string());
    if let Some(attempt) = flow
        .engine
        .chatgpt_attempts
        .lock()
        .await
        .get_mut(&flow.attempt)
    {
        attempt.status = if message.is_none() {
            "complete"
        } else {
            "failed"
        }
        .into();
        attempt.message = message.clone();
    }
    flow.done.notify_one();
    let code = if message.is_none() {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };
    (code, Html(page(message.as_deref())))
}

async fn complete(flow: &Flow, params: &HashMap<String, String>) -> Result<()> {
    if let Some(error) = params.get("error_description").or(params.get("error")) {
        bail!("{error}");
    }
    let code = params.get("code").context("missing authorization code")?;
    ensure!(
        params.get("state") == Some(&flow.state),
        "invalid OAuth state"
    );
    let returned = params.get("client_id");
    if let (Some(saved), Some(returned)) = (&flow.saved, returned) {
        ensure!(
            saved == returned,
            "ChatGPT returned a different client than this connection. Connect again."
        );
    }
    let client_id = flow
        .saved
        .clone()
        .or_else(|| returned.cloned())
        .context("ChatGPT sign-in did not return a client ID. Connect again.")?;
    let tokens = token_request(&[
        ("grant_type", "authorization_code"),
        ("client_id", &client_id),
        ("code", code),
        ("code_verifier", &flow.verifier),
        ("redirect_uri", &flow.redirect),
        ("resource", RESOURCE),
    ])
    .await?;
    ensure!(
        tokens["scope"]
            .as_str()
            .is_some_and(|scope| scope.split(' ').any(|scope| scope == SCOPE)),
        "ChatGPT sign-in finished without token sharing. Sign in again and allow token sharing, or connect OpenAI with an API key."
    );
    ensure!(
        tokens["refresh_token"].is_string(),
        "ChatGPT sign-in did not return a refresh token."
    );
    verify(
        tokens["id_token"]
            .as_str()
            .context("ChatGPT sign-in did not return an ID token.")?,
        &client_id,
        &flow.nonce,
    )?;
    let engine = &flow.engine;
    keep(&tokens, &mut *engine.chatgpt_token.lock().await).await?;
    // One OpenAI connection at a time: ChatGPT replaces a saved API key and vice versa.
    let id = engine
        .store
        .desktop_meta("credentials")?
        .into_iter()
        .find(|entry| entry["integrationID"] == "openai")
        .and_then(|entry| entry["id"].as_str().map(str::to_owned))
        .unwrap_or_else(|| session_store::id("cred"));
    let entry = json!({"id":id,"integrationID":"openai","label":"ChatGPT","active":true,"value":{"type":"external","methodID":METHOD,"metadata":{"clientID":client_id}}});
    engine
        .store
        .desktop_put_meta("credentials", &id, Some(&entry))?;
    crate::desktop_models::notify_credentials(&engine.desktop_events, "openai", Some(&id));
    Ok(())
}

async fn status(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> ApiResult {
    let mut attempts = engine.chatgpt_attempts.lock().await;
    let attempt = attempts
        .get_mut(&id)
        .context("sign-in attempt not found")
        .map_err(error)?;
    if attempt.status == "pending" && session_store::now() >= attempt.expires {
        attempt.status = "expired".into();
        attempt.done.notify_one();
    }
    let mut result = json!({"status":attempt.status,"time":{"created":attempt.created,"expires":attempt.expires}});
    if let Some(message) = &attempt.message {
        result["message"] = json!(message);
    }
    located(&query, result)
}

async fn cancel(State(engine): State<Arc<Engine>>, Path(id): Path<String>) -> StatusCode {
    if let Some(attempt) = engine.chatgpt_attempts.lock().await.remove(&id) {
        attempt.done.notify_one();
    }
    StatusCode::NO_CONTENT
}

async fn token_request(form: &[(&str, &str)]) -> Result<Value> {
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .post(format!("{ISSUER}/api/accounts/oauth/token"))
        .form(form)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("could not reach OpenAI sign-in"))?;
    // Never surface token endpoint bodies: they can contain credentials.
    ensure!(
        response.status().is_success(),
        "OpenAI sign-in returned HTTP {}",
        response.status()
    );
    Ok(response.json().await?)
}

async fn keep(tokens: &Value, cached: &mut Option<(String, u64)>) -> Result<String> {
    let access = tokens["access_token"]
        .as_str()
        .context("ChatGPT sign-in did not return an access token.")?
        .to_owned();
    if let Some(refresh) = tokens["refresh_token"].as_str().map(str::to_owned) {
        tokio::task::spawn_blocking(move || Credentials::set("openai", &refresh)).await??;
    }
    *cached = Some((
        access.clone(),
        session_store::now() + tokens["expires_in"].as_u64().unwrap_or(3600) * 1000,
    ));
    Ok(access)
}

/// The ID token arrives directly from the issuer's token endpoint over TLS, which OpenID Connect
/// Core 3.1.3.7 accepts in place of a signature check; its claims still bind it to this attempt.
fn verify(token: &str, client_id: &str, nonce: &str) -> Result<()> {
    let claims: Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD.decode(token.split('.').nth(1).context("invalid ID token")?)?,
    )
    .context("invalid ID token")?;
    ensure!(claims["iss"] == ISSUER, "ID token issuer is not OpenAI");
    ensure!(
        claims["aud"] == client_id
            || claims["aud"]
                .as_array()
                .is_some_and(|audience| audience.iter().any(|value| value == client_id)),
        "ID token is for a different client"
    );
    ensure!(
        claims["nonce"] == nonce,
        "ID token nonce does not match this sign-in attempt"
    );
    ensure!(
        claims["exp"]
            .as_u64()
            .is_some_and(|exp| exp * 1000 > session_store::now()),
        "ID token has expired"
    );
    ensure!(
        claims["sub"]
            .as_str()
            .is_some_and(|subject| !subject.trim().is_empty()),
        "ID token subject is missing"
    );
    Ok(())
}

/// One stable host ID per installation, as upstream sends with every authorization.
fn host(engine: &Engine) -> Result<String> {
    if let Some(host) = engine
        .store
        .desktop_meta("chatgpt")?
        .into_iter()
        .find(|entry| entry["id"] == "host")
        .and_then(|entry| entry["value"].as_str().map(str::to_owned))
    {
        return Ok(host);
    }
    let host = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    engine
        .store
        .desktop_put_meta("chatgpt", "host", Some(&json!({"id":"host","value":host})))?;
    Ok(host)
}

/// Random bytes from v4 UUIDs (122 random bits each), which the engine already depends on.
fn random(length: usize) -> Vec<u8> {
    (0..length.div_ceil(16))
        .flat_map(|_| *uuid::Uuid::new_v4().as_bytes())
        .take(length)
        .collect()
}

fn page(error: Option<&str>) -> String {
    let (title, body) = match error {
        None => (
            "ChatGPT connected",
            "You can close this window and return to Cryzo.".to_owned(),
        ),
        Some(error) => (
            "ChatGPT sign-in failed",
            error
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;"),
        ),
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>body{{font-family:system-ui,sans-serif;display:grid;place-items:center;min-height:100vh;margin:0}}main{{max-width:420px;padding:24px}}</style></head><body><main><h1>{title}</h1><p>{body}</p></main></body></html>"
    )
}
