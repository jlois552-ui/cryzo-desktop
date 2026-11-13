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

/// Epoch milliseconds for `YYYY-MM-DD`, or for an RFC 3339 date-time that carries its UTC offset
/// (`Z` or `±HH:MM`). Uses Howard Hinnant's days_from_civil; the engine has no date library.
pub fn epoch_ms(value: &str) -> Option<i64> {
    let (date, time) = value.split_once('T').unwrap_or((value, ""));
    let mut parts = date.splitn(3, '-').map(str::parse::<i64>);
    let (year, month, day) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let year_of_era = shifted - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100
        + day_of_year
        - 719_468;
    if time.is_empty() {
        return Some(days * 86_400_000);
    }
    let (clock, offset) = match time.strip_suffix('Z') {
        Some(clock) => (clock, 0),
        None => {
            let (clock, zone) = time.split_at(time.rfind(['+', '-'])?);
            let (hours, minutes) = zone[1..].split_once(':')?;
            let minutes = hours.parse::<i64>().ok()? * 60 + minutes.parse::<i64>().ok()?;
            (
                clock,
                if zone.starts_with('-') {
                    -minutes
                } else {
                    minutes
                },
            )
        }
    };
    let mut clock = clock.split(':');
    let hour = clock.next()?.parse::<i64>().ok()?;
    let minute = clock.next()?.parse::<i64>().ok()?;
    let second = clock
        .next()
        .map_or(Some(0.0), |value| value.parse::<f64>().ok())? as i64;
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(((days * 86_400) + hour * 3600 + minute * 60 + second - offset * 60) * 1000)
}

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
        .route("/api/market/media/file", axum::routing::get(file))
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

/// Streams one stored media file so the renderer can show it as a `blob:` URL under the desktop's
/// content policy. Only the configured deployment's own storage URLs are fetched.
async fn file(
    State(engine): State<Arc<Engine>>,
    RawQuery(query): RawQuery,
) -> Result<axum::response::Response, (StatusCode, Json<Value>)> {
    if !free_tier::signed_in(&engine).map_err(|error| reject(error.into()))? {
        return Err(reject(Error::SignIn));
    }
    let requested = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(name, _)| name == "url")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| reject(Error::Request("missing media URL".into())))?;
    let requested = url::Url::parse(&requested)
        .map_err(|_| reject(Error::Request("invalid media URL".into())))?;
    let site = url::Url::parse(&free_tier::gateway().map_err(|error| reject(error.into()))?)
        .map_err(|_| reject(Error::Request("invalid gateway URL".into())))?;
    // Convex serves storage from the deployment's .convex.cloud host; its HTTP actions use .convex.site.
    let storage = site
        .host_str()
        .and_then(|host| host.strip_suffix(".convex.site"))
        .map(|name| format!("{name}.convex.cloud"));
    if requested.scheme() != "https"
        || requested.host_str() != storage.as_deref()
        || !requested.path().starts_with("/api/storage/")
    {
        return Err(reject(Error::Request(
            "media is not from Cryzo cloud".into(),
        )));
    }
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|error| reject(Error::Request(error.to_string())))?
        .get(requested)
        .send()
        .await
        .map_err(|_| reject(Error::Request("could not load media".into())))?;
    if !response.status().is_success() {
        return Err(reject(Error::Request("could not load media".into())));
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.starts_with("image/") || value.starts_with("video/"))
        .ok_or_else(|| reject(Error::Request("not an image or video".into())))?
        .to_owned();
    axum::response::Response::builder()
        .header("content-type", content_type)
        .header("cache-control", "private, max-age=3600")
        .body(axum::body::Body::from_stream(response.bytes_stream()))
        .map_err(|error| reject(Error::Request(error.to_string())))
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
