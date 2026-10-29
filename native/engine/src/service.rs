use crate::{
    catalog::Catalog,
    credentials::Credentials,
    provider::{Message, chat},
    store::Store,
    tools::{Operation, Workspace},
};
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

pub struct Engine {
    _ownership: std::fs::File,
    pub store: Store,
    pub catalog: Catalog,
    pub(crate) running: Mutex<HashMap<String, CancellationToken>>,
    pub(crate) permissions: Mutex<HashMap<String, crate::native_sessions::PendingPermission>>,
    pub(crate) desktop_events: tokio::sync::broadcast::Sender<Value>,
    pub ptys: crate::pty::PtyManager,
    pub(crate) login_attempts: crate::free_tier::Attempts,
    pub(crate) chatgpt_attempts: crate::chatgpt::Attempts,
    pub(crate) chatgpt_token: crate::chatgpt::Token,
}

impl Engine {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        // Recovery may reset running claims only after the previous process has released ownership.
        let ownership = std::fs::File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        ownership
            .try_lock()
            .map_err(|error| anyhow::anyhow!("cannot acquire local engine ownership: {error}"))?;
        let desktop_events = tokio::sync::broadcast::channel(1024).0;
        let mut catalog = Catalog::load()?;
        crate::free_tier::configure(&mut catalog);
        Ok(Self {
            _ownership: ownership,
            store: Store::open(path)?,
            catalog,
            login_attempts: Mutex::new(HashMap::new()),
            chatgpt_attempts: Mutex::new(HashMap::new()),
            chatgpt_token: Mutex::new(None),
            running: Mutex::new(HashMap::new()),
            permissions: Mutex::new(HashMap::new()),
            ptys: crate::pty::PtyManager::new(desktop_events.clone()),
            desktop_events,
        })
    }

    pub async fn cancel(&self, session: &str) {
        if let Some(cancel) = self.running.lock().await.get(session) {
            cancel.cancel();
        }
    }

    pub async fn generate(
        self: Arc<Self>,
        session: String,
        provider: String,
        model: String,
        id: String,
        text: String,
    ) -> Result<bool> {
        ensure!(
            !text.trim().is_empty() && text.len() <= 1_000_000,
            "prompt must contain between 1 and 1,000,000 bytes"
        );
        let endpoint = self.catalog.chat_endpoint(&provider, &model)?.to_owned();
        let secret = Credentials::get(&provider)?;
        let cancel = CancellationToken::new();
        {
            let mut running = self.running.lock().await;
            ensure!(
                !running.contains_key(&session),
                "session is running; wait or stop it before sending another prompt"
            );
            // Admission never calls a provider. The durable inbox is written first.
            let admitted =
                self.store
                    .admit(&session, &id, &json!({"role":"user","content":text}))?;
            if !admitted {
                return Ok(false);
            }
            self.store.claim(&session)?;
            running.insert(session.clone(), cancel.clone());
        }
        let engine = self.clone();
        tokio::spawn(async move {
            let result = engine
                .step(&session, &id, &endpoint, &secret, &model, cancel.clone())
                .await;
            let status = if cancel.is_cancelled() {
                "interrupted"
            } else if result.is_err() {
                "failed"
            } else {
                "idle"
            };
            if let Err(error) = result {
                let _ = engine.store.append(
                    &session,
                    "generation.failed",
                    &json!({"message":error.to_string()}),
                );
            }
            let _ = engine.store.finish(&session, status);
            engine.running.lock().await.remove(&session);
        });
        Ok(true)
    }

    async fn step(
        &self,
        session: &str,
        id: &str,
        endpoint: &str,
        secret: &str,
        model: &str,
        cancel: CancellationToken,
    ) -> Result<()> {
        self.store
            .deliver(session, id)?
            .context("prompt was already delivered")?;
        let mut events = Vec::new();
        let mut after = 0;
        loop {
            let page = self.store.events(session, after)?;
            if page.is_empty() {
                break;
            }
            after = page.last().unwrap().sequence;
            events.extend(page);
        }
        let messages = events
            .iter()
            .filter(|event| matches!(event.kind.as_str(), "message.user" | "message.assistant"))
            .map(|event| serde_json::from_value::<Message>(event.data.clone()))
            .collect::<serde_json::Result<Vec<_>>>()?;
        let (send, mut receive) = mpsc::channel(64);
        let generation = chat(endpoint, secret, model, &messages, send, cancel);
        tokio::pin!(generation);
        loop {
            tokio::select! {
                delta = receive.recv() => {
                    if let Some(text) = delta { self.store.append(session,"message.delta",&json!({"text":text}))?; }
                }
                result = &mut generation => {
                    let content = result?;
                    while let Ok(text) = receive.try_recv() { self.store.append(session,"message.delta",&json!({"text":text}))?; }
                    self.store.append(session,"message.assistant",&json!({"role":"assistant","content":content}))?;
                    return Ok(());
                }
            }
        }
    }

    pub async fn tool(
        &self,
        session: &str,
        operation: Operation,
        approval: Option<&str>,
    ) -> Result<Value> {
        let workspace = Workspace::open(std::path::Path::new(
            &self.store.session(session)?.directory,
        ))?;
        if operation.needs_approval() && approval.is_none() {
            let operation = serde_json::to_value(&operation)?;
            let id = self.store.request_approval(session, &operation)?;
            self.store.append(
                session,
                "permission.requested",
                &json!({"id":id,"operation":operation}),
            )?;
            return Ok(json!({"approval_required":id,"operation":operation}));
        }
        let result = workspace
            .execute(&self.store, session, &operation, approval)
            .await;
        match &result {
            Ok(output) => {
                self.store.append(
                    session,
                    "tool.completed",
                    &json!({"operation":operation,"output":output}),
                )?;
            }
            Err(error) => {
                self.store.append(
                    session,
                    "tool.failed",
                    &json!({"operation":operation,"message":error.to_string()}),
                )?;
            }
        }
        result
    }
}

#[derive(Clone)]
struct App {
    engine: Arc<Engine>,
    token: String,
}

pub fn router(engine: Arc<Engine>, token: String) -> Router {
    let desktop = crate::desktop_api::router(engine.clone());
    let state = App { engine, token };
    Router::new()
        .route("/v0/health", get(health))
        .route("/v0/catalog", get(catalog))
        .route("/v0/session", get(sessions).post(create_session))
        .route("/v0/session/{session}/events", get(events))
        .route("/v0/session/{session}/prompt", post(prompt))
        .route("/v0/session/{session}/cancel", post(cancel))
        .route("/v0/session/{session}/tool", post(tool))
        .route("/v0/session/{session}/approval/{approval}", post(approval))
        .merge(desktop.with_state(()))
        .layer(axum::extract::DefaultBodyLimit::max(3_000_000))
        .layer(middleware::from_fn_with_state(state.clone(), authorize))
        .with_state(state)
}

async fn authorize(
    State(state): State<App>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some(&format!("Bearer {}", state.token))
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"unauthorized"})),
        )
            .into_response();
    }
    next.run(request).await
}

type ApiResult = std::result::Result<Json<Value>, ApiError>;
struct ApiError(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(error: E) -> Self {
        Self(error.into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":self.0.to_string()})),
        )
            .into_response()
    }
}

async fn health() -> Json<Value> {
    Json(
        json!({"engine":"cryzo-rust","version":env!("CARGO_PKG_VERSION"),"upstream_compatible":false}),
    )
}
async fn catalog(State(state): State<App>) -> ApiResult {
    Ok(Json(serde_json::to_value(&state.engine.catalog)?))
}
async fn sessions(State(state): State<App>) -> ApiResult {
    Ok(Json(serde_json::to_value(state.engine.store.sessions()?)?))
}
#[derive(Deserialize)]
struct CreateSession {
    directory: String,
    title: String,
}
async fn create_session(State(state): State<App>, Json(input): Json<CreateSession>) -> ApiResult {
    ensure_api(input.title.len() <= 500, "title is too long")?;
    Ok(Json(serde_json::to_value(
        state
            .engine
            .store
            .create_session(std::path::Path::new(&input.directory), &input.title)?,
    )?))
}
#[derive(Deserialize)]
struct Cursor {
    #[serde(default)]
    after: i64,
}
async fn events(
    State(state): State<App>,
    Path(session): Path<String>,
    Query(cursor): Query<Cursor>,
) -> ApiResult {
    Ok(Json(serde_json::to_value(
        state.engine.store.events(&session, cursor.after)?,
    )?))
}
#[derive(Deserialize)]
struct Prompt {
    id: String,
    provider: String,
    model: String,
    text: String,
}
async fn prompt(
    State(state): State<App>,
    Path(session): Path<String>,
    Json(input): Json<Prompt>,
) -> ApiResult {
    Ok(Json(
        json!({"admitted":state.engine.generate(session,input.provider,input.model,input.id,input.text).await?}),
    ))
}
async fn cancel(State(state): State<App>, Path(session): Path<String>) -> ApiResult {
    state.engine.cancel(&session).await;
    Ok(Json(json!({"cancelled":true})))
}
#[derive(Deserialize)]
struct Tool {
    operation: Operation,
    approval: Option<String>,
}
async fn tool(
    State(state): State<App>,
    Path(session): Path<String>,
    Json(input): Json<Tool>,
) -> ApiResult {
    Ok(Json(
        state
            .engine
            .tool(&session, input.operation, input.approval.as_deref())
            .await?,
    ))
}
#[derive(Deserialize)]
struct Decision {
    allow: bool,
}
async fn approval(
    State(state): State<App>,
    Path((session, approval)): Path<(String, String)>,
    Json(input): Json<Decision>,
) -> ApiResult {
    state
        .engine
        .store
        .decide(&session, &approval, input.allow)?;
    Ok(Json(json!({"recorded":true})))
}
fn ensure_api(condition: bool, message: &str) -> Result<()> {
    ensure!(condition, "{message}");
    Ok(())
}
