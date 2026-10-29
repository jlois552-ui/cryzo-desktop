//! First-party device login. NVIDIA's owner key never enters the desktop process.
use crate::{
    catalog::{Catalog, Provider},
    credentials::Credentials,
    service::Engine,
    session_store,
};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    extract::{Path, RawQuery, State},
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::Mutex;

/// Listed in picker order; the renderer defaults to the first. Nemotron 3 Ultra answered
/// reliably on NVIDIA's free queue when DeepSeek and GLM were stalling.
pub const MODELS: &[&str] = &[
    "nvidia/nemotron-3-ultra-550b-a55b",
    "deepseek-ai/deepseek-v4.1-flash",
    "z-ai/glm-5.3-flash",
    "minimaxai/minimax-m3.1-flash",
    "nvidia/nemotron-3.5-lightning-30b-a3b",
    "nvidia/nemotron-3.5-super-vl-120b-a12b",
];

pub struct Attempt {
    code: String,
    created: u64,
    expires: u64,
    next_poll: Instant,
    status: String,
}
pub type Attempts = Mutex<HashMap<String, Attempt>>;

pub fn gateway() -> Result<String> {
    let value = std::env::var("CRYZO_GATEWAY_URL").ok()
        .or_else(||option_env!("CRYZO_GATEWAY_URL").map(str::to_owned))
        .context("Cryzo Free is not configured in this development build. Personal provider keys are available through Connect provider.")?;
    let url = url::Url::parse(&value)?;
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))),
        "gateway must use HTTPS"
    );
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "invalid gateway URL"
    );
    Ok(value.trim_end_matches('/').to_owned())
}

pub const SIGN_IN_REQUIRED: &str = "Sign in with Google to use Cryzo Free models.";

/// Free models are listed before sign-in; running one needs a token issued by this gateway.
pub fn signed_in(engine: &Engine) -> Result<bool> {
    let origin = json!(gateway()?);
    Ok(engine
        .store
        .desktop_meta("credentials")?
        .iter()
        .any(|entry| {
            entry["integrationID"] == "cryzo"
                && entry["active"] == true
                && entry.pointer("/value/metadata/gateway") == Some(&origin)
        }))
}

pub fn configure(catalog: &mut Catalog) {
    let Some(nvidia) = catalog.providers.iter().find(|p| p.id == "nvidia") else {
        return;
    };
    // Each curated model is its own family, so the picker shows all of them rather than only
    // the newest Nemotron.
    let models = MODELS
        .iter()
        .filter_map(|id| nvidia.models.iter().find(|m| m.id == *id))
        .cloned()
        .map(|mut model| {
            model.details["family"] = json!(model.id);
            model
        })
        .collect();
    catalog.providers.insert(
        0,
        Provider {
            id: "cryzo".into(),
            name: "Cryzo Free".into(),
            credential_env: vec![],
            models,
        },
    );
}

pub fn endpoint(catalog: &Catalog, provider: &str, model: &str) -> Result<String> {
    let endpoint = catalog
        .native_api(provider, model)?
        .base_url
        .as_deref()
        .context("provider URL missing")?;
    if provider == "cryzo" {
        return Ok(format!("{}/v1", gateway()?));
    }
    Ok(endpoint.to_owned())
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(20))
        .build()?)
}

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn error(error: impl ToString) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"_tag":"InvalidRequestError","message":error.to_string()})),
    )
}
pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/api/integration/cryzo/connect/oauth", post(begin))
        .route(
            "/api/integration/cryzo/connect/oauth/{attempt}",
            get(status).delete(cancel),
        )
        .with_state(engine)
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
    if input["methodID"] != "google" {
        return Err(error("unknown authentication method"));
    }
    let base = gateway().map_err(error)?;
    let response = client()
        .map_err(error)?
        .post(format!("{base}/api/auth/device/code"))
        .json(&json!({"client_id":"cryzo-desktop","scope":"inference"}))
        .send()
        .await
        .map_err(|_| error("cannot reach Cryzo sign-in"))?;
    if !response.status().is_success() {
        return Err(error(format!(
            "sign-in returned HTTP {}",
            response.status()
        )));
    }
    let data: Value = response
        .json()
        .await
        .map_err(|_| error("invalid sign-in response"))?;
    let code = data["device_code"]
        .as_str()
        .context("device authorization missing")
        .map_err(error)?
        .to_owned();
    let user_code = data["user_code"]
        .as_str()
        .context("verification code missing")
        .map_err(error)?;
    let url = format!(
        "{base}/device?user_code={}",
        url::form_urlencoded::byte_serialize(user_code.as_bytes()).collect::<String>()
    );
    let id = session_store::id("con");
    let created = session_store::now();
    let expires = created + data["expires_in"].as_u64().unwrap_or(600).min(1800) * 1000;
    let mut attempts = engine.login_attempts.lock().await;
    attempts.retain(|_, attempt| attempt.expires > created);
    ensure_capacity(&attempts).map_err(error)?;
    attempts.insert(
        id.clone(),
        Attempt {
            code,
            created,
            expires,
            next_poll: Instant::now(),
            status: "pending".into(),
        },
    );
    located(
        &query,
        json!({"attemptID":id,"url":url,"instructions":format!("Sign in with Google and approve code {user_code} to connect this desktop."),"mode":"auto","time":{"created":created,"expires":expires}}),
    )
}
fn ensure_capacity(attempts: &HashMap<String, Attempt>) -> Result<()> {
    ensure!(attempts.len() < 8, "too many pending sign-ins");
    Ok(())
}
async fn status(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> ApiResult {
    let mut attempts = engine.login_attempts.lock().await;
    let attempt = attempts
        .get_mut(&id)
        .context("sign-in attempt not found")
        .map_err(error)?;
    if session_store::now() >= attempt.expires {
        attempt.status = "expired".into();
        attempt.code.clear();
    }
    if attempt.status == "pending" && Instant::now() >= attempt.next_poll {
        attempt.next_poll = Instant::now() + std::time::Duration::from_secs(5);
        let base = gateway().map_err(error)?;
        let response=client().map_err(error)?.post(format!("{base}/api/auth/device/token"))
            .json(&json!({"client_id":"cryzo-desktop","device_code":attempt.code,"grant_type":"urn:ietf:params:oauth:grant-type:device_code"}))
            .send().await.map_err(|_|error("cannot reach Cryzo sign-in"))?;
        let data: Value = response
            .json()
            .await
            .map_err(|_| error("invalid sign-in response"))?;
        if let Some(token) = data["access_token"].as_str() {
            // Reconcile availability before publishing a connected provider.
            let catalog = client()
                .map_err(error)?
                .get(format!("{base}/v1/models"))
                .bearer_auth(token)
                .send()
                .await
                .map_err(|_| error("cannot load free models"))?;
            if !catalog.status().is_success() {
                return Err(error("cannot load free models"));
            }
            let catalog: Value = catalog
                .json()
                .await
                .map_err(|_| error("invalid model catalog"))?;
            let ids: Vec<_> = catalog["data"]
                .as_array()
                .context("invalid model catalog")
                .map_err(error)?
                .iter()
                .filter_map(|m| m["id"].as_str())
                .filter(|id| MODELS.contains(id))
                .collect();
            let metadata = json!({"id":"available","models":ids});
            Credentials::set("cryzo", token).map_err(error)?;
            engine
                .store
                .desktop_put_meta("cryzo", "available", Some(&metadata))
                .map_err(error)?;
            let credential = json!({"id":"cred_cryzo","integrationID":"cryzo","label":"Google account","active":true,"value":{"type":"external","methodID":"google","metadata":{"gateway":base}}});
            engine
                .store
                .desktop_put_meta("credentials", "cred_cryzo", Some(&credential))
                .map_err(error)?;
            attempt.status = "complete".into();
            attempt.code.clear();
            for kind in [
                "integration.updated",
                "provider.updated",
                "model.updated",
                "credential.updated",
            ] {
                let _ = engine
                    .desktop_events
                    .send(crate::desktop_api::event(kind, json!({})));
            }
        } else {
            match data["error"].as_str() {
                Some("authorization_pending") => {}
                Some("slow_down") => {
                    attempt.next_poll = Instant::now() + std::time::Duration::from_secs(10)
                }
                Some("expired_token") => {
                    attempt.status = "expired".into();
                    attempt.code.clear()
                }
                _ => {
                    attempt.status = "failed".into();
                    attempt.code.clear()
                }
            }
        }
    }
    let mut result = json!({"status":attempt.status,"time":{"created":attempt.created,"expires":attempt.expires}});
    if attempt.status == "failed" {
        result["message"] = json!("Sign-in was denied or could not be completed. Please reconnect.")
    }
    located(&query, result)
}
async fn cancel(State(engine): State<Arc<Engine>>, Path(id): Path<String>) -> StatusCode {
    engine.login_attempts.lock().await.remove(&id);
    StatusCode::NO_CONTENT
}

pub fn validate_variant(catalog: &Catalog, reference: &Value) -> Result<()> {
    let provider = reference["providerID"]
        .as_str()
        .context("provider missing")?;
    let id = reference["id"].as_str().context("model missing")?;
    let model = catalog
        .providers
        .iter()
        .find(|p| p.id == provider)
        .and_then(|p| p.models.iter().find(|m| m.id == id))
        .context("unknown model")?;
    if let Some(variant) = reference["variant"].as_str()
        && !crate::desktop_models::variants(model)
            .iter()
            .any(|v| v["id"] == variant)
    {
        bail!("unsupported reasoning variant")
    }
    Ok(())
}
