//! Dictation for the composer's mic button: the recording goes to the Cryzo gateway, which
//! transcribes it with Whisper on Cloudflare. Nothing is kept here.
use crate::remote;
use axum::{Json, Router, http::StatusCode, routing::post};
use serde_json::{Value, json};

pub fn router() -> Router {
    Router::new().route("/api/cryzo/transcribe", post(transcribe))
}

/// `{audio, seconds}` is a base64 recording as the browser made it and its length; the answer is
/// `{text}`.
async fn transcribe(Json(input): Json<Value>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let failed = |error: anyhow::Error| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"_tag":"InvalidRequestError","message":format!("{error:#}")})),
        )
    };
    let account = remote::account().map_err(failed)?;
    let data = remote::call(
        reqwest::Method::POST,
        "voice/transcribe",
        Some(account),
        json!({"audio":input["audio"],"seconds":input["seconds"]}),
    )
    .await
    .map_err(failed)?;
    Ok(Json(json!({"text":data["text"]})))
}
