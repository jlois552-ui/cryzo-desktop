//! Frozen OpenCode HTTP contracts implemented by the Rust engine.
//! Only register implemented routes. Missing capabilities must not return fake success.
use crate::service::Engine;
use axum::{
    Json, Router,
    extract::{Path, RawQuery, State},
    http::StatusCode,
    routing::get,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

pub fn router(engine: Arc<Engine>) -> Router {
    let models = crate::desktop_models::router(engine.clone());
    let sessions = crate::native_sessions::router(engine.clone());
    let files = crate::desktop_files::router(engine.clone());
    let free = crate::free_tier::router(engine.clone());
    let chatgpt = crate::chatgpt::router(engine.clone());
    let market = crate::market::router(engine.clone());
    Router::new()
        .route("/api/info", get(info))
        .route("/api/location", get(location))
        .route("/api/config", get(config))
        .route("/api/project", get(projects))
        .route("/api/event", get(events))
        .route("/api/pty", get(pty_list).post(pty_create))
        .route(
            "/api/pty/{id}",
            get(pty_get).put(pty_update).delete(pty_remove),
        )
        .route(
            "/api/pty/{id}/connect-token",
            axum::routing::post(pty_ticket),
        )
        .with_state(engine)
        .merge(models)
        .merge(sessions)
        .merge(files)
        .merge(free)
        .merge(chatgpt)
        .merge(market)
}

pub(crate) fn directory(query: Option<&str>) -> anyhow::Result<PathBuf> {
    let directory = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(name, _)| name == "location[directory]")
        .filter(|(_, value)| !value.is_empty())
        .map(|(_, value)| PathBuf::from(value.as_ref()))
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
        })
        .ok_or_else(|| anyhow::anyhow!("no project directory"))?
        .canonicalize()?;
    anyhow::ensure!(directory.is_dir(), "location is not a directory");
    Ok(directory)
}
fn located(directory: &std::path::Path, data: Value) -> Json<Value> {
    Json(
        json!({"location":{"directory":directory.to_string_lossy().trim_start_matches(r"\\?\")},"data":data}),
    )
}
/// The native engine reads no configuration documents yet, so a Location truthfully has none.
/// The renderer resolves the composer's model selection only after this list loads.
async fn config(RawQuery(query): RawQuery) -> ApiResult {
    directory(query.as_deref()).map_err(invalid)?;
    Ok(Json(json!([])))
}
async fn pty_list(State(engine): State<Arc<Engine>>, RawQuery(query): RawQuery) -> ApiResult {
    let directory = directory(query.as_deref()).map_err(invalid)?;
    Ok(located(
        &directory,
        json!(engine.ptys.list(&directory).map_err(invalid)?),
    ))
}
async fn pty_create(
    State(engine): State<Arc<Engine>>,
    RawQuery(query): RawQuery,
    Json(input): Json<Value>,
) -> ApiResult {
    let directory = directory(query.as_deref()).map_err(invalid)?;
    let cwd = directory.clone();
    let info = tokio::task::spawn_blocking(move || engine.ptys.create(&cwd, &input))
        .await
        .map_err(invalid)?
        .map_err(invalid)?;
    Ok(located(&directory, info))
}
async fn pty_get(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> ApiResult {
    let directory = directory(query.as_deref()).map_err(invalid)?;
    Ok(located(
        &directory,
        engine.ptys.get(&id, &directory).map_err(invalid)?.info(),
    ))
}
async fn pty_update(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
    Json(input): Json<Value>,
) -> ApiResult {
    let directory = directory(query.as_deref()).map_err(invalid)?;
    let pty = engine.ptys.get(&id, &directory).map_err(invalid)?;
    let info = tokio::task::spawn_blocking(move || pty.update(&input))
        .await
        .map_err(invalid)?
        .map_err(invalid)?;
    let mut updated = event("pty.updated", json!({"info":info}));
    updated["location"] =
        json!({"directory":directory.to_string_lossy().trim_start_matches(r"\\?\")});
    let _ = engine.desktop_events.send(updated);
    Ok(located(&directory, info))
}
async fn pty_remove(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let directory = directory(query.as_deref()).map_err(invalid)?;
    tokio::task::spawn_blocking(move || engine.ptys.remove(&id, &directory))
        .await
        .map_err(invalid)?
        .map_err(invalid)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn pty_ticket(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
    headers: axum::http::HeaderMap,
) -> ApiResult {
    if headers
        .get("x-opencode-ticket")
        .and_then(|value| value.to_str().ok())
        != Some("1")
    {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"_tag":"ForbiddenError"})),
        ));
    }
    let directory = directory(query.as_deref()).map_err(invalid)?;
    let ticket = engine.ptys.ticket(&id, &directory).map_err(invalid)?;
    Ok(located(
        &directory,
        json!({"ticket":ticket,"expires_in":60}),
    ))
}

async fn info() -> Json<Value> {
    Json(
        json!({"version":env!("CARGO_PKG_VERSION"),"pid":std::process::id(),
        "urls":[],"paths":{"tmp":std::env::temp_dir().to_string_lossy()},
        "capabilities":{"persistentPty":false}}),
    )
}

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;

fn invalid(error: impl ToString) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"_tag":"InvalidRequestError","message":error.to_string()})),
    )
}

async fn location(State(engine): State<Arc<Engine>>, RawQuery(query): RawQuery) -> ApiResult {
    let directory = url::form_urlencoded::parse(query.as_deref().unwrap_or_default().as_bytes())
        .find(|(name, _)| name == "location[directory]")
        .filter(|(_, value)| !value.is_empty())
        .map(|(_, value)| PathBuf::from(value.as_ref()));
    let explicit = directory.is_some();
    let directory = directory
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
        })
        .ok_or_else(|| invalid("home directory is unavailable"))?;
    let directory = directory.canonicalize().map_err(invalid)?;
    if !directory.is_dir() {
        return Err(invalid("location must be a directory"));
    }
    let directory = directory
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    let id = if explicit {
        let project = engine
            .store
            .desktop_project(std::path::Path::new(&directory))
            .map_err(invalid)?;
        let _ = engine
            .desktop_events
            .send(event("project.updated", project.clone()));
        project["id"].clone()
    } else {
        json!("global")
    };
    Ok(Json(
        json!({"directory":directory,"project":{"id":id,"directory":directory,"canonical":directory}}),
    ))
}

async fn projects(State(engine): State<Arc<Engine>>) -> ApiResult {
    Ok(Json(json!(
        engine.store.desktop_projects().map_err(invalid)?
    )))
}

pub(crate) fn event(kind: &str, data: Value) -> Value {
    json!({"id":format!("evt_{}", uuid::Uuid::new_v4()),"created":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,"type":kind,"data":data})
}

async fn events(State(engine): State<Arc<Engine>>) -> impl axum::response::IntoResponse {
    use axum::response::sse::{Event, KeepAlive, Sse};
    let receiver = engine.desktop_events.subscribe();
    let stream =
        futures_util::stream::unfold((receiver, true), |(mut receiver, first)| async move {
            let value = if first {
                event("server.connected", json!({}))
            } else {
                match receiver.recv().await {
                    Ok(value) => value,
                    // Close on a gap. The upstream client reconnects and reloads its projections.
                    Err(_) => return None,
                }
            };
            Some((
                Ok::<_, std::convert::Infallible>(Event::default().data(value.to_string())),
                (receiver, false),
            ))
        });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
