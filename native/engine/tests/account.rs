use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

// Its own test binary: the gateway origin is process-wide configuration.
#[tokio::test]
async fn the_sidebar_account_is_signed_out_without_contacting_the_cloud() {
    // A closed port: a request that escaped the sign-in check would not answer with signedIn false.
    // SAFETY: set before the engine starts and before any other thread reads the environment.
    unsafe { std::env::set_var("CRYZO_GATEWAY_URL", "http://127.0.0.1:9") };
    let temp = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(
        cryzo_engine::service::Engine::open(&temp.path().join("account.sqlite")).unwrap(),
    );
    let app = cryzo_engine::service::router(engine, "fixture-key".into());
    let request = axum::http::Request::builder()
        .uri("/api/cryzo/account")
        .header("authorization", "Bearer fixture-key")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"signedIn": false})
    );
}
