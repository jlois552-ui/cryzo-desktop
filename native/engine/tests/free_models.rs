use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

// Its own test binary: the gateway origin is process-wide configuration.
#[tokio::test]
async fn free_models_are_listed_before_sign_in_and_prompting_them_asks_for_it() {
    // SAFETY: set before the engine starts and before any other thread reads the environment.
    unsafe { std::env::set_var("CRYZO_GATEWAY_URL", "https://gateway.invalid") };
    let temp = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(
        cryzo_engine::service::Engine::open(&temp.path().join("free.sqlite")).unwrap(),
    );
    let app = cryzo_engine::service::router(engine.clone(), "fixture-key".into());
    let call = |method: &str, uri: String, body: Option<Value>| {
        let app = app.clone();
        let request = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", "Bearer fixture-key")
            .header("content-type", "application/json")
            .body(body.map_or_else(axum::body::Body::empty, |body| {
                axum::body::Body::from(body.to_string())
            }))
            .unwrap();
        async move {
            let response = app.oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };

    let (status, models) = call("GET", "/api/model?location%5Bdirectory%5D=".into(), None).await;
    assert_eq!(status, 200);
    let free: Vec<_> = models["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|model| model["providerID"] == "cryzo")
        .filter_map(|model| model["id"].as_str())
        .collect();
    assert_eq!(free, cryzo_engine::free_tier::MODELS);
    // The renderer hides models it reads as old and not the newest of their family.
    let free: Vec<_> = models["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|model| model["providerID"] == "cryzo")
        .collect();
    assert_eq!(free[0]["id"], "nvidia/nemotron-3-ultra-550b-a55b");
    assert_eq!(free[0]["time"]["released"], 1_780_531_200_000_i64); // 2026-06-04
    assert!(free.iter().all(|model| model["family"] == model["id"]));
    let (_, providers) = call("GET", "/api/provider?location%5Bdirectory%5D=".into(), None).await;
    assert!(
        providers["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "cryzo")
    );
    let (_, default) = call(
        "GET",
        "/api/model/default?location%5Bdirectory%5D=".into(),
        None,
    )
    .await;
    assert_eq!(default["data"]["providerID"], "cryzo");

    // The composer holds every send until the Location's configuration list loads.
    let (status, config) = call("GET", "/api/config?location%5Bdirectory%5D=".into(), None).await;
    assert_eq!((status.as_u16(), config), (200, json!([])));

    let model = json!({"providerID":"cryzo","id":cryzo_engine::free_tier::MODELS[0]});
    let (status, _) = call(
        "POST",
        "/api/session".into(),
        Some(json!({"id":"ses_free","location":{"directory":temp.path()},"model":model})),
    )
    .await;
    assert!(status.is_success());
    let (status, error) = call(
        "POST",
        "/api/session/ses_free/prompt".into(),
        Some(json!({"id":"msg_free","text":"Hello"})),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(error["_tag"], "InvalidRequestError");
    assert_eq!(error["kind"], "cryzo_sign_in");
    // Rejected before admission: nothing waits to run after a later sign-in.
    assert_eq!(
        engine.store.desktop_state("ses_free").unwrap()["inbox"],
        json!([])
    );
}
