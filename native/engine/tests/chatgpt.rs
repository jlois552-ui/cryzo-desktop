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

#[tokio::test]
async fn openai_offers_chatgpt_sign_in_and_rejects_a_forged_callback() {
    let temp = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(
        cryzo_engine::service::Engine::open(&temp.path().join("chatgpt.sqlite")).unwrap(),
    );
    let app = cryzo_engine::service::router(engine, "fixture-key".into());

    let (_, integrations) = call(
        &app,
        "GET",
        "/api/integration?location%5Bdirectory%5D=",
        None,
    )
    .await;
    let integrations = integrations["data"].as_array().unwrap();
    let openai = integrations.iter().find(|i| i["id"] == "openai").unwrap();
    assert_eq!(
        openai["methods"][1],
        json!({"id":"chatgpt-token-sharing","type":"oauth","label":"Sign in with ChatGPT"})
    );
    // No Cryzo route exists for OpenCode's subscriptions or Copilot's OAuth.
    for hidden in ["opencode", "opencode-go", "github-copilot"] {
        assert!(
            integrations.iter().all(|i| i["id"] != hidden),
            "{hidden} offered"
        );
    }

    let (status, begun) = call(
        &app,
        "POST",
        "/api/integration/openai/connect/oauth?location%5Bdirectory%5D=",
        Some(json!({"methodID":"chatgpt-token-sharing"})),
    )
    .await;
    assert_eq!(status, 200);
    let url = url::Url::parse(begun["data"]["url"].as_str().unwrap()).unwrap();
    assert_eq!(
        url.as_str().split('?').next(),
        Some("https://auth.openai.com/api/accounts/authorize")
    );
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], "dynamic_agent_client");
    assert_eq!(query["agent_name_hint"], "Cryzo");
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["resource"], "https://api.openai.com/v1");
    assert!(
        query["scope"]
            .split(' ')
            .any(|scope| scope == "chatgpt.tokens.use.direct")
    );
    assert!(query["ext_agent_host_id"].starts_with("urn:uuid:"));
    let redirect = &query["redirect_uri"];
    assert!(redirect.starts_with("http://127.0.0.1:") && redirect.ends_with("/auth/callback"));

    // A callback without this attempt's state fails without contacting OpenAI.
    let forged = reqwest::get(format!("{redirect}?code=stolen&state=wrong"))
        .await
        .unwrap();
    assert_eq!(forged.status().as_u16(), 400);
    let attempt = begun["data"]["attemptID"].as_str().unwrap();
    let (_, status) = call(
        &app,
        "GET",
        &format!("/api/integration/openai/connect/oauth/{attempt}?location%5Bdirectory%5D="),
        None,
    )
    .await;
    assert_eq!(status["data"]["status"], "failed");
    assert_eq!(status["data"]["message"], "invalid OAuth state");
}

#[tokio::test]
async fn a_chatgpt_connection_lists_only_plan_models_at_no_cost() {
    let temp = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(
        cryzo_engine::service::Engine::open(&temp.path().join("plan.sqlite")).unwrap(),
    );
    // Metadata only: the OS credential store is not touched.
    engine
        .store
        .desktop_put_meta(
            "credentials",
            "cred_openai",
            Some(&json!({"id":"cred_openai","integrationID":"openai","label":"ChatGPT","active":true,
                "value":{"type":"external","methodID":"chatgpt-token-sharing","metadata":{"clientID":"client_fixture"}}})),
        )
        .unwrap();
    let app = cryzo_engine::service::router(engine, "fixture-key".into());
    let (_, models) = call(&app, "GET", "/api/model?location%5Bdirectory%5D=", None).await;
    let openai: Vec<_> = models["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|model| model["providerID"] == "openai")
        .collect();
    assert!(!openai.is_empty());
    assert!(openai.iter().all(|model| {
        cryzo_engine::chatgpt::MODELS.contains(&model["id"].as_str().unwrap())
            && model["cost"] == json!([])
    }));
    let (_, integration) = call(
        &app,
        "GET",
        "/api/integration/openai?location%5Bdirectory%5D=",
        None,
    )
    .await;
    assert_eq!(integration["data"]["connections"][0]["method"], "oauth");
}
