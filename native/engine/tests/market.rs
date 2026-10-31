use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    let request = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", "Bearer fixture-key")
        .header("content-type", "application/json")
        .body(body.map_or_else(axum::body::Body::empty, |body| {
            axum::body::Body::from(body.to_string())
        }))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

// Its own test binary: the gateway origin is process-wide configuration.
#[tokio::test]
async fn marketing_asks_for_cryzo_sign_in_before_contacting_the_cloud() {
    // A closed port: any request that escaped the sign-in check would fail differently.
    // SAFETY: set before the engine starts and before any other thread reads the environment.
    unsafe { std::env::set_var("CRYZO_GATEWAY_URL", "http://127.0.0.1:9") };
    let temp = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(
        cryzo_engine::service::Engine::open(&temp.path().join("market.sqlite")).unwrap(),
    );
    let app = cryzo_engine::service::router(engine, "fixture-key".into());
    for (method, uri, body) in [
        ("GET", "/api/market/accounts", None),
        (
            "POST",
            "/api/market/posts/create",
            Some(json!({"content":"Launch"})),
        ),
        (
            "POST",
            "/api/market/media/upload",
            Some(json!({"path": temp.path().join("clip.mp4")})),
        ),
    ] {
        let (status, error) = call(&app, method, uri, body).await;
        assert_eq!(status, 400, "{uri}");
        assert_eq!(error["kind"], "cryzo_sign_in", "{uri}");
    }
    let (_, error) = call(
        &app,
        "POST",
        "/api/market/media/upload",
        Some(json!({"path":"notes.txt"})),
    )
    .await;
    assert_eq!(
        error["message"],
        "Use a PNG, JPEG, GIF or WebP image, or an MP4, MOV or WebM video."
    );
}
