use crate::{
    catalog::{Catalog, Model},
    credentials::Credentials,
    service::Engine,
    session_store,
};
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::{Path, RawQuery, State},
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn available(catalog: &Catalog) -> impl Iterator<Item = (&crate::catalog::Provider, &Model)> {
    catalog.providers.iter().flat_map(|provider| {
        provider
            .models
            .iter()
            .filter(|model| {
                model.apis.iter().any(|api| {
                    [
                        "chat-completions",
                        "responses",
                        "messages",
                        "generate-content",
                    ]
                    .contains(&api.protocol.as_str())
                        && api.base_url.is_some()
                })
            })
            .map(move |model| (provider, model))
    })
}
pub fn model_info(provider: &str, model: &Model, enabled: bool) -> Value {
    let data = &model.details;
    let variants = variants(model);
    let cost = match (
        data.pointer("/cost/text/input/price"),
        data.pointer("/cost/text/output/price"),
    ) {
        (Some(input), Some(output)) if input.is_number() && output.is_number() => {
            json!([{"input":input,"output":output,"cache":{"read":data.pointer("/cost/text/input/cache/read").and_then(Value::as_f64).unwrap_or(0.0),"write":0}}])
        }
        _ => json!([]),
    };
    // Catalog "YYYY-MM-DD" as epoch milliseconds (days_from_civil). The renderer hides models that are
    // neither recent nor the newest of their family, so an unknown date must not read as 1970.
    let released = data["release_date"].as_str().and_then(|date| {
        let mut parts = date.splitn(3, '-').map(str::parse::<i64>);
        let (year, month, day) = (
            parts.next()?.ok()?,
            parts.next()?.ok()?,
            parts.next()?.ok()?,
        );
        let year = if month <= 2 { year - 1 } else { year };
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        Some((era * 146_097 + day_of_era - 719_468) * 86_400_000)
    });
    json!({"id":model.id,"modelID":model.id,"providerID":provider,"name":model.name,"type":"chat","family":data["family"].as_str().unwrap_or(&model.id),
        "capabilities":{"tools":data.pointer("/capabilities/tools/supported").and_then(Value::as_bool).unwrap_or(false),"input":data.pointer("/modalities/input").cloned().unwrap_or(json!(["text"])),"output":data.pointer("/modalities/output").cloned().unwrap_or(json!(["text"]))},
        "variants":variants,"time":{"released":released.unwrap_or(0)},"cost":cost,
        "status":"active","enabled":enabled,"limit":{"context":data.pointer("/limit/context").and_then(Value::as_u64).unwrap_or(200000),
            "output":data.pointer("/limit/output").and_then(Value::as_u64).unwrap_or(32000)}})
}
pub fn variants(model: &Model) -> Vec<Value> {
    if let Some(efforts) = model
        .details
        .pointer("/capabilities/reasoning/effort")
        .and_then(Value::as_array)
    {
        return efforts
            .iter()
            .filter_map(Value::as_str)
            .map(|effort| json!({"id":effort,"settings":{"reasoningEffort":effort}}))
            .collect();
    }
    if model.details.pointer("/capabilities/reasoning/toggle") == Some(&json!(true)) {
        return vec![
            json!({"id":"off","settings":{"thinking":false}}),
            json!({"id":"on","settings":{"thinking":true}}),
        ];
    }
    vec![]
}

/// Apply only advertised variants; switching agent modes does not change this choice.
pub fn apply_variant(model: &Model, variant: Option<&str>, request: &mut Value) -> Result<()> {
    let Some(variant) = variant else {
        return Ok(());
    };
    ensure!(
        variants(model).iter().any(|v| v["id"] == variant),
        "reasoning variant is not supported by this model"
    );
    let openrouter = model
        .apis
        .iter()
        .filter_map(|api| api.base_url.as_deref())
        .any(|url| url.starts_with("https://openrouter.ai/"));
    if openrouter {
        request["reasoning"] = if ["on", "off"].contains(&variant) {
            json!({"enabled":variant=="on"})
        } else {
            json!({"effort":variant})
        };
    } else if ["on", "off"].contains(&variant) {
        request["chat_template_kwargs"] = json!({"enable_thinking":variant == "on"});
    } else {
        request["reasoning_effort"] = json!(variant);
    }
    Ok(())
}
pub fn models(engine: &Engine) -> Result<Vec<Value>> {
    let connected = engine.store.desktop_meta("credentials")?;
    // Like OpenCode's free catalog, Cryzo Free is offered before sign-in; prompting asks for it.
    let free = crate::free_tier::gateway().is_ok();
    // A ChatGPT plan serves only its own models, at no per-token cost to the user.
    let chatgpt = connected.iter().any(|entry| {
        entry["integrationID"] == "openai"
            && entry["active"] == true
            && entry["value"]["methodID"] == crate::chatgpt::METHOD
    });
    Ok(available(&engine.catalog)
        .filter(|(provider, _)| provider.id != "cryzo" || free)
        .filter(|(provider, model)| {
            provider.id != "openai"
                || !chatgpt
                || crate::chatgpt::MODELS.contains(&model.id.as_str())
        })
        .filter(|(provider, model)| {
            provider.id != "nvidia"
                || connected
                    .iter()
                    .find(|entry| entry["integrationID"] == "nvidia" && entry["active"] == true)
                    .and_then(|entry| entry.pointer("/value/metadata/models"))
                    .and_then(Value::as_array)
                    .is_none_or(|ids| ids.iter().any(|id| id == &model.id))
        })
        .filter(|(_, model)| model.details["type"].as_str().unwrap_or("chat") == "chat")
        .map(|(provider, model)| {
            let mut info = model_info(
                &provider.id,
                model,
                provider.id == "cryzo"
                    || connected.iter().any(|entry| {
                        entry["integrationID"] == provider.id && entry["active"] == true
                    }),
            );
            if chatgpt && provider.id == "openai" {
                info["cost"] = json!([]);
            }
            info
        })
        .collect())
}
pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/api/model", get(model_list))
        .route("/api/model/default", get(default_model))
        .route("/api/provider", get(providers))
        .route("/api/provider/{id}", get(provider_get))
        .route("/api/integration", get(integrations))
        .route("/api/integration/{id}", get(integration_get))
        .route("/api/integration/{id}/connect/key", post(connect_key))
        .route("/api/credential", get(credentials))
        .route(
            "/api/credential/{id}",
            axum::routing::delete(disconnect).patch(rename_credential),
        )
        .route("/api/agent", get(agents))
        .route("/api/agent/{id}", get(agent_get))
        .with_state(engine)
}
type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn error(error: impl ToString) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"_tag":"InvalidRequestError","message":error.to_string()})),
    )
}
fn located(query: &Option<String>, data: Value) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = crate::desktop_api::directory(query.as_deref()).map_err(error)?;
    Ok(Json(
        json!({"location":{"directory":path.to_string_lossy().trim_start_matches(r"\\?\")},"data":data}),
    ))
}
async fn model_list(State(engine): State<Arc<Engine>>, RawQuery(query): RawQuery) -> ApiResult {
    located(
        &query,
        json!(
            models(&engine)
                .map_err(error)?
                .into_iter()
                .filter(|model| model["enabled"] == true)
                .collect::<Vec<_>>()
        ),
    )
}
async fn default_model(State(engine): State<Arc<Engine>>, RawQuery(query): RawQuery) -> ApiResult {
    // A personal key's models come first; Cryzo Free is the default only without one.
    let models = models(&engine).map_err(error)?;
    let enabled = || models.iter().filter(|model| model["enabled"] == true);
    let model = enabled()
        .find(|model| model["providerID"] != "cryzo")
        .or_else(|| enabled().next())
        .cloned();
    let mut response = located(&query, Value::Null)?;
    if let Some(model) = model {
        response.0["data"] = model;
    }
    Ok(response)
}
/// OpenCode's own subscriptions and GitHub Copilot's OAuth have no Cryzo route, so they are not offered.
const UNOFFERED: &[&str] = &["opencode", "opencode-go", "github-copilot"];
fn provider_values(engine: &Engine) -> Vec<Value> {
    engine.catalog.providers.iter().filter(|provider|!UNOFFERED.contains(&provider.id.as_str())&&available(&engine.catalog).any(|(p,_)|p.id==provider.id)).map(|p|json!({"id":p.id,"integrationID":p.id,"name":p.name,"activation":"auto","package":"@ai-sdk/openai-compatible"})).collect()
}
async fn providers(State(engine): State<Arc<Engine>>, RawQuery(query): RawQuery) -> ApiResult {
    let connected = engine.store.desktop_meta("credentials").map_err(error)?;
    let free = crate::free_tier::gateway().is_ok();
    located(
        &query,
        json!(
            provider_values(&engine)
                .into_iter()
                .filter(|provider| (free && provider["id"] == "cryzo")
                    || connected
                        .iter()
                        .any(|entry| entry["active"] == true
                            && entry["integrationID"] == provider["id"]))
                .collect::<Vec<_>>()
        ),
    )
}
async fn provider_get(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> ApiResult {
    located(
        &query,
        provider_values(&engine)
            .into_iter()
            .find(|p| p["id"] == id)
            .ok_or_else(|| error("provider not found"))?,
    )
}
fn integration_values(engine: &Engine) -> Result<Vec<Value>> {
    let connected = engine.store.desktop_meta("credentials")?;
    Ok(provider_values(engine).into_iter().map(|p|{
    let connections:Vec<_>=connected.iter().filter(|entry|entry["integrationID"]==p["id"]&&entry["active"]==true).map(|entry|json!({"id":entry["id"],"type":"credential","label":entry["label"],"method":if entry["value"]["methodID"]=="native-keyring"{"key"}else{"oauth"}})).collect();
    let methods=match p["id"].as_str(){
        Some("cryzo")=>json!([{"id":"google","type":"oauth","label":"Sign in with Google"}]),
        Some("openai")=>json!([{"type":"key","label":"API key"},{"id":crate::chatgpt::METHOD,"type":"oauth","label":"Sign in with ChatGPT"}]),
        _=>json!([{"type":"key","label":"API key"}]),
    };
    json!({"id":p["id"],"name":p["name"],"methods":methods,"connections":connections})}).collect())
}
async fn integrations(State(engine): State<Arc<Engine>>, RawQuery(query): RawQuery) -> ApiResult {
    located(&query, json!(integration_values(&engine).map_err(error)?))
}
async fn integration_get(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> ApiResult {
    located(
        &query,
        integration_values(&engine)
            .map_err(error)?
            .into_iter()
            .find(|p| p["id"] == id)
            .ok_or_else(|| error("integration not found"))?,
    )
}
pub fn save_key(
    engine: &Engine,
    provider: &str,
    secret: &str,
    label: Option<&str>,
) -> Result<Value> {
    ensure!(
        available(&engine.catalog).any(|(p, _)| p.id == provider),
        "unsupported provider transport"
    );
    ensure!(provider != "cryzo", "Cryzo Free requires Google sign-in");
    Credentials::set(provider, secret)?;
    let existing = engine
        .store
        .desktop_meta("credentials")?
        .into_iter()
        .find(|entry| entry["integrationID"] == provider);
    let id = existing
        .as_ref()
        .and_then(|entry| entry["id"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| session_store::id("cred"));
    // Return an external OS-store reference. The renderer never receives the saved key.
    let entry = json!({"id":id,"integrationID":provider,"label":label.unwrap_or("API key"),"active":true,"value":{"type":"external","methodID":"native-keyring"}});
    engine
        .store
        .desktop_put_meta("credentials", &id, Some(&entry))?;
    Ok(entry)
}
async fn connect_key(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    Json(input): Json<Value>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let key = input["key"]
        .as_str()
        .ok_or_else(|| error("key is required"))?
        .to_owned();
    let label = input["label"].as_str().map(str::to_owned);
    if id == "cryzo" {
        return Err(error("Cryzo Free requires Google sign-in"));
    }
    let validation = validate_key(&engine.catalog, &id, &key)
        .await
        .map_err(error)?;
    // An API key replaces a ChatGPT sign-in in the same credential slot.
    if id == "openai" {
        *engine.chatgpt_token.lock().await = None;
    }
    let events = engine.desktop_events.clone();
    let provider = id.clone();
    let entry = tokio::task::spawn_blocking(move || {
        let mut entry = save_key(&engine, &id, &key, label.as_deref())?;
        entry["value"]["metadata"] = validation;
        engine.store.desktop_put_meta(
            "credentials",
            entry["id"].as_str().context("credential ID missing")?,
            Some(&entry),
        )?;
        Ok::<_, anyhow::Error>(entry)
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    notify_credentials(&events, &provider, entry["id"].as_str());
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn notify_credentials(
    events: &tokio::sync::broadcast::Sender<Value>,
    provider: &str,
    active: Option<&str>,
) {
    let _ = events.send(crate::desktop_api::event("credential.updated", json!({})));
    // This global event refreshes models in every already-open Location, including
    // global Settings and project composers. Location-less model.updated is ignored.
    let _ = events.send(crate::desktop_api::event(
        "credential.switched",
        json!({"integrationID":provider,"credentialID":active}),
    ));
}

async fn rename_credential(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    Json(input): Json<Value>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let label = input["label"]
        .as_str()
        .map(str::trim)
        .filter(|label| !label.is_empty() && label.len() <= 200)
        .ok_or_else(|| error("credential label must contain between 1 and 200 bytes"))?;
    let mut entry = engine
        .store
        .desktop_meta("credentials")
        .map_err(error)?
        .into_iter()
        .find(|entry| entry["id"] == id)
        .ok_or_else(|| error("credential not found"))?;
    entry["label"] = json!(label);
    engine
        .store
        .desktop_put_meta("credentials", &id, Some(&entry))
        .map_err(error)?;
    let _ = engine
        .desktop_events
        .send(crate::desktop_api::event("credential.updated", json!({})));
    Ok(StatusCode::NO_CONTENT)
}

async fn validate_key(catalog: &Catalog, provider: &str, key: &str) -> Result<Value> {
    ensure!(
        !key.trim().is_empty() && key.len() <= 16_384 && !key.contains(['\r', '\n']),
        "invalid API key format"
    );
    let p = catalog
        .providers
        .iter()
        .find(|p| p.id == provider)
        .context("unknown provider")?;
    if !["nvidia", "openrouter", "openai", "anthropic", "google"].contains(&provider) {
        return Ok(json!({"validation":"saved-unverified"}));
    }
    let api = p
        .models
        .iter()
        .find_map(|m| catalog.native_api(provider, &m.id).ok())
        .context("unsupported provider transport")?;
    let endpoint = api.base_url.as_deref().context("provider URL missing")?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let path = if provider == "openrouter" {
        "key"
    } else {
        "models"
    };
    let request = client.get(format!("{}/{path}", endpoint.trim_end_matches('/')));
    let request = match api.protocol.as_str() {
        "messages" => request
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01"),
        "generate-content" => request.header("x-goog-api-key", key),
        _ => request.bearer_auth(key),
    };
    let response = request.send().await.map_err(|_| {
        anyhow::anyhow!(
            "could not validate provider credentials; reconnect when the provider is reachable"
        )
    })?;
    ensure!(
        ![401, 403].contains(&response.status().as_u16()),
        "provider rejected this API key or its permissions"
    );
    if !response.status().is_success() {
        return Ok(json!({"validation":"saved-unverified"}));
    }
    // NVIDIA's models endpoint is public: catalog access does not verify a key.
    if provider == "nvidia" {
        let data: Value = response.json().await.context("invalid NVIDIA catalog")?;
        let models = data["data"]
            .as_array()
            .context("invalid NVIDIA catalog")?
            .iter()
            .filter_map(|m| m["id"].as_str())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        return Ok(json!({"validation":"saved-unverified","models":models}));
    }
    Ok(json!({"validation":"verified"}))
}
async fn credentials(State(engine): State<Arc<Engine>>) -> ApiResult {
    Ok(Json(
        json!({"data":engine.store.desktop_meta("credentials").map_err(error)?}),
    ))
}
async fn disconnect(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let entry = engine
        .store
        .desktop_meta("credentials")
        .map_err(error)?
        .into_iter()
        .find(|entry| entry["id"] == id)
        .ok_or_else(|| error("credential not found"))?;
    let provider = entry["integrationID"]
        .as_str()
        .context("credential owner missing")
        .map_err(error)?
        .to_owned();
    if provider == "cryzo"
        && let Ok(base) = crate::free_tier::gateway()
    {
        if entry.pointer("/value/metadata/gateway") != Some(&json!(base)) {
            return Err(error(
                "restore the original gateway configuration before disconnecting this account",
            ));
        }
        let token = tokio::task::spawn_blocking(|| Credentials::get("cryzo"))
            .await
            .map_err(error)?
            .map_err(error)?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(error)?;
        // A disconnected device must stop authorizing requests before forgetting its token.
        let response = client
            .post(format!("{base}/api/auth/sign-out"))
            .bearer_auth(token)
            .header("origin", &base)
            .json(&json!({}))
            .send()
            .await
            .map_err(|_| error("could not revoke the desktop connection; retry disconnect"))?;
        if !response.status().is_success() && response.status() != StatusCode::UNAUTHORIZED {
            return Err(error(
                "could not revoke the desktop connection; retry disconnect",
            ));
        }
    }
    if provider == "openai" {
        *engine.chatgpt_token.lock().await = None;
    }
    let credential_provider = provider.clone();
    tokio::task::spawn_blocking(move || Credentials::remove(&credential_provider))
        .await
        .map_err(error)?
        .map_err(error)?;
    engine
        .store
        .desktop_put_meta("credentials", &id, None)
        .map_err(error)?;
    notify_credentials(&engine.desktop_events, &provider, None);
    Ok(StatusCode::NO_CONTENT)
}
pub fn agent(id: &str) -> Option<Value> {
    if !["build", "plan"].contains(&id) {
        return None;
    }
    Some(
        json!({"id":id,"name":id,"request":{"settings":{},"headers":{},"body":{}},"mode":"primary","hidden":false,"permissions":[{"action":"*","resource":"*","effect":"ask"}]}),
    )
}
async fn agents(RawQuery(query): RawQuery) -> ApiResult {
    located(
        &query,
        json!([agent("build").unwrap(), agent("plan").unwrap()]),
    )
}
async fn agent_get(Path(id): Path<String>, RawQuery(query): RawQuery) -> ApiResult {
    located(&query, agent(&id).ok_or_else(|| error("agent not found"))?)
}
