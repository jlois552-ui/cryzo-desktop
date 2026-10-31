//! Marketing: forwards the renderer's requests to the Cryzo cloud `/market` API with the
//! desktop's Cryzo sign-in. Tokens stay in the OS credential store and never reach the renderer.
use crate::{credentials::Credentials, free_tier, service::Engine};
use anyhow::Context;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, RawQuery, State},
    http::{Method, StatusCode},
    routing::{any, post},
};
use serde_json::{Value, json};
use std::sync::Arc;

pub const SIGN_IN_REQUIRED: &str = "Sign in with Google to use Marketing.";

pub enum Error {
    SignIn,
    Request(String),
}

impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        Error::Request(error.to_string())
    }
}

/// One `/market/{path}` call. Returns the response's `data`.
pub async fn call(
    engine: &Engine,
    method: Method,
    path: &str,
    query: Option<&str>,
    body: Option<Value>,
) -> Result<Value, Error> {
    if !free_tier::signed_in(engine)? {
        return Err(Error::SignIn);
    }
    if path.is_empty()
        || path.len() > 100
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'/' || byte == b'-')
    {
        return Err(Error::Request("unknown marketing request".into()));
    }
    let token = tokio::task::spawn_blocking(|| Credentials::get("cryzo"))
        .await
        .context("credential store unavailable")??;
    let mut url = format!("{}/market/{path}", free_tier::gateway()?);
    if let Some(query) = query.filter(|query| !query.is_empty()) {
        url = format!("{url}?{query}");
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // Analytics refreshes query every connected network in one request.
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(anyhow::Error::from)?;
    let request = client.request(method, &url).bearer_auth(token);
    let request = match body {
        Some(body) => request.json(&body),
        None => request,
    };
    let response = request.send().await.map_err(|_| {
        Error::Request("Could not reach Cryzo cloud. Check your connection.".into())
    })?;
    if response.status() == StatusCode::UNAUTHORIZED {
        return Err(Error::SignIn);
    }
    let status = response.status();
    let data: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(Error::Request(
            data["error"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Marketing request failed (HTTP {})", status.as_u16())),
        ));
    }
    Ok(data["data"].clone())
}

pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/api/market/media/upload", post(upload))
        .route("/api/market/{*path}", any(proxy))
        .with_state(engine)
}

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;

fn reject(error: Error) -> (StatusCode, Json<Value>) {
    let body = match error {
        Error::SignIn => {
            json!({"_tag":"InvalidRequestError","message":SIGN_IN_REQUIRED,"kind":"cryzo_sign_in"})
        }
        Error::Request(message) => json!({"_tag":"InvalidRequestError","message":message}),
    };
    (StatusCode::BAD_REQUEST, Json(body))
}

async fn proxy(
    State(engine): State<Arc<Engine>>,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
    method: Method,
    body: Bytes,
) -> ApiResult {
    let body = match method {
        Method::GET => None,
        _ if body.is_empty() => Some(json!({})),
        _ => Some(
            serde_json::from_slice(&body)
                .map_err(|_| reject(Error::Request("invalid request body".into())))?,
        ),
    };
    let data = call(&engine, method, &path, query.as_deref(), body)
        .await
        .map_err(reject)?;
    Ok(Json(json!({ "data": data })))
}

/// Uploads a local image or video the user picked to Cryzo cloud storage and adds it to the
/// media library. The file is streamed, so large videos never sit in memory.
async fn upload(State(engine): State<Arc<Engine>>, Json(input): Json<Value>) -> ApiResult {
    let path = std::path::PathBuf::from(
        input["path"]
            .as_str()
            .ok_or_else(|| reject(Error::Request("choose a file".into())))?,
    );
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let content_type = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => {
            return Err(reject(Error::Request(
                "Use a PNG, JPEG, GIF or WebP image, or an MP4, MOV or WebM video.".into(),
            )));
        }
    };
    if !free_tier::signed_in(&engine).map_err(|error| reject(error.into()))? {
        return Err(reject(Error::SignIn));
    }
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| reject(Error::Request("could not open the file".into())))?;
    let size = file
        .metadata()
        .await
        .map_err(|_| reject(Error::Request("could not read the file".into())))?
        .len();
    if !path.is_absolute() || size == 0 || size > 1024 * 1024 * 1024 {
        return Err(reject(Error::Request(
            "Choose a file between 1 byte and 1 GB.".into(),
        )));
    }
    let target = call(
        &engine,
        Method::POST,
        "media/upload-url",
        None,
        Some(json!({})),
    )
    .await
    .map_err(reject)?;
    let target = target
        .as_str()
        .ok_or_else(|| reject(Error::Request("Cryzo cloud returned no upload URL".into())))?
        .to_owned();
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1800))
        .build()
        .map_err(|error| reject(Error::Request(error.to_string())))?
        .post(target)
        .header("content-type", content_type)
        .header("content-length", size)
        .body(reqwest::Body::wrap_stream(
            tokio_util::io::ReaderStream::new(file),
        ))
        .send()
        .await
        .map_err(|_| reject(Error::Request("The upload was interrupted.".into())))?;
    let stored: Value = response
        .json()
        .await
        .map_err(|_| reject(Error::Request("Cryzo cloud rejected the upload.".into())))?;
    let storage_id = stored["storageId"]
        .as_str()
        .ok_or_else(|| reject(Error::Request("Cryzo cloud rejected the upload.".into())))?;
    let name = input["name"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    call(
        &engine,
        Method::POST,
        "media/register",
        None,
        Some(json!({"storageId":storage_id,"name":name})),
    )
    .await
    .map_err(reject)?;
    Ok(Json(json!({"data":{"storageId":storage_id}})))
}
