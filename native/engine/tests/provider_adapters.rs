use axum::{Json, Router, http::HeaderMap, routing::post};
use cryzo_engine::{
    catalog::Catalog,
    desktop_models::{apply_variant, model_info},
    provider::{self, Delta},
    wire,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

#[tokio::test]
async fn byok_connections_populate_models_and_support_renaming_without_exposing_keys() {
    let temp = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(
        cryzo_engine::service::Engine::open(&temp.path().join("providers.sqlite")).unwrap(),
    );
    let app = cryzo_engine::service::router(engine.clone(), "fixture-key".into());
    // Public metadata references the OS store; no test touches real credentials.
    for provider in ["nvidia", "openrouter", "openai", "anthropic", "google"] {
        let id = format!("cred_{provider}");
        engine
            .store
            .desktop_put_meta(
                "credentials",
                &id,
                Some(&json!({
                    "id":id,"integrationID":provider,"label":"Personal key","active":true,
                    "value":{"type":"external","methodID":"native-keyring"}
                })),
            )
            .unwrap();
    }
    for endpoint in ["provider", "model"] {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/{endpoint}?location%5Bdirectory%5D="))
                    .header("authorization", "Bearer fixture-key")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let data: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        for provider in ["nvidia", "openrouter", "openai", "anthropic", "google"] {
            let field = if endpoint == "model" {
                "providerID"
            } else {
                "id"
            };
            assert!(
                data["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row[field] == provider),
                "{provider} missing from {endpoint}"
            );
        }
    }
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("PATCH")
                .uri("/api/credential/cred_nvidia")
                .header("authorization", "Bearer fixture-key")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({"label":"My NVIDIA"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/integration/nvidia?location%5Bdirectory%5D=")
                .header("authorization", "Bearer fixture-key")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let data: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["data"]["connections"][0]["label"], "My NVIDIA");
    assert_eq!(data["data"]["connections"][0]["method"], "key");
    assert!(data["data"]["connections"][0].get("key").is_none());
}

#[tokio::test]
async fn switching_plan_build_keeps_the_selected_model_and_effort() {
    let temp = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(
        cryzo_engine::service::Engine::open(&temp.path().join("settings.sqlite")).unwrap(),
    );
    let app = cryzo_engine::service::router(engine, "fixture-key".into());
    let model =
        json!({"providerID":"nvidia","id":cryzo_engine::free_tier::MODELS[0],"variant":"high"});
    let input = json!({"id":"ses_selection","location":{"directory":temp.path()},"model":model});
    for (path, input) in [
        ("/api/session", input),
        ("/api/session/ses_selection/agent", json!({"agent":"plan"})),
        ("/api/session/ses_selection/agent", json!({"agent":"build"})),
    ] {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("authorization", "Bearer fixture-key")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(input.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
    }
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/session/ses_selection")
                .header("authorization", "Bearer fixture-key")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let data: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["data"]["model"], model);
}

#[test]
fn nvidia_variants_modalities_and_unknown_cost_remain_truthful() {
    let mut catalog = Catalog::load().unwrap();
    cryzo_engine::free_tier::configure(&mut catalog);
    let nvidia = catalog.providers.iter().find(|p| p.id == "nvidia").unwrap();
    let model = nvidia
        .models
        .iter()
        .find(|m| m.id == "deepseek-ai/deepseek-v4.1-flash")
        .unwrap();
    let info = model_info("nvidia", model, true);
    assert!(
        info["variants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["id"] == "high")
    );
    assert!(
        info["capabilities"]["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "image/png")
    );
    assert_eq!(
        model.apis[0].compatibility["reasoning_field"],
        "reasoning_content"
    );
    let mut body = json!({});
    apply_variant(model, Some("high"), &mut body).unwrap();
    assert_eq!(body["reasoning_effort"], "high");
    assert!(apply_variant(model, Some("medium"), &mut body).is_err());
    let mut unknown = model.clone();
    unknown.details.as_object_mut().unwrap().remove("cost");
    assert_eq!(model_info("nvidia", &unknown, true)["cost"], json!([]));
    let free = catalog.providers.iter().find(|p| p.id == "cryzo").unwrap();
    assert_eq!(free.models.len(), 6);
    assert!(
        catalog
            .chat_endpoint("cryzo", cryzo_engine::free_tier::MODELS[0])
            .is_err()
    );
    assert!(
        free.models
            .iter()
            .all(|m| m.id != "z-ai/glm-5.3" && !m.name.contains("Pickle"))
    );
    assert!(catalog.native_api("openai", "gpt-5.6-cyber").is_ok());
}

#[test]
fn encrypted_compatible_reasoning_is_reassembled_without_duplicating_metadata() {
    let mut decoder = wire::Decoder::default();
    for data in ["signed-", "reasoning"] {
        decoder.event("chat-completions",json!({"choices":[{"delta":{"reasoning_details":[{"index":0,"type":"reasoning.encrypted","id":"rs_1","data":data}]}}]})).unwrap();
    }
    let native = decoder.native("chat-completions");
    assert_eq!(native["chat-completions"][0]["data"], "signed-reasoning");
    assert_eq!(native["chat-completions"][0]["id"], "rs_1");
}

#[tokio::test]
async fn native_transports_stream_reasoning_text_tools_and_provider_replay_data() {
    for protocol in [
        "chat-completions",
        "responses",
        "messages",
        "generate-content",
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let path = match protocol {
            "responses" => "/v1/responses",
            "messages" => "/v1/messages",
            "generate-content" => "/v1/models/fixture:streamGenerateContent",
            _ => "/v1/chat/completions",
        };
        let app=Router::new().route(path,post(move |headers:HeaderMap,Json(body):Json<Value>|async move{
            match protocol {"messages"=>assert_eq!(headers["x-api-key"],"fixture-key"),"generate-content"=>assert_eq!(headers["x-goog-api-key"],"fixture-key"),_=>assert_eq!(headers["authorization"],"Bearer fixture-key")};
            assert!(body.get("cryzo_protocol").is_none());
            if protocol!="generate-content"{assert_eq!(body["stream"],true);}
            let events=match protocol {
                "responses"=>vec![json!({"type":"response.reasoning_summary_text.delta","delta":"Think 🦀"}),json!({"type":"response.output_item.done","item":{"type":"reasoning","id":"rs_1","encrypted_content":"fixture-signature"}}),json!({"type":"response.output_text.delta","delta":"Hello"}),json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call_1","name":"read"}}),json!({"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"path\":\"a.txt\"}"}),json!({"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":3}}})],
                "messages"=>vec![json!({"type":"message_start","message":{"usage":{"input_tokens":10}}}),json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Think 🦀"}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"fixture-signature"}}),json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hello"}}),json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"call_1","name":"read","input":{}}}),json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a.txt\"}"}}),json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":3}}),json!({"type":"message_stop"})],
                "generate-content"=>vec![json!({"candidates":[{"content":{"parts":[{"text":"Think 🦀","thought":true,"thoughtSignature":"fixture-signature"},{"text":"Hello"},{"functionCall":{"id":"call_1","name":"read","args":{"path":"a.txt"}},"thoughtSignature":"tool-signature"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":3}})],
                _=>vec![json!({"choices":[{"delta":{"reasoning_content":"Think 🦀"}}]}),json!({"choices":[{"delta":{"content":"Hello","tool_calls":[{"index":0,"id":"call_1","function":{"name":"read","arguments":"{\"path\":\"a.txt\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":3}})],
            };
            ([("content-type","text/event-stream")],events.iter().map(|e|format!("data: {e}\n\n")).collect::<String>())
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (send, mut receive) = mpsc::channel(16);
        let result=provider::completion_events(&endpoint,"fixture-key",json!({"model":"fixture","messages":[{"role":"user","content":"Hi"}],"cryzo_protocol":protocol}),send,CancellationToken::new()).await.unwrap();
        assert_eq!(result.text, "Hello");
        assert_eq!(result.reasoning, "Think 🦀");
        assert_eq!(result.calls.len(), 1);
        assert_eq!(result.calls[0].name, "read");
        assert_eq!(result.calls[0].arguments, "{\"path\":\"a.txt\"}");
        assert_eq!(result.usage["prompt_tokens"], 10);
        assert!(matches!(receive.recv().await, Some(Delta::Reasoning(_))));
        if protocol != "chat-completions" {
            assert!(!result.native[protocol].as_array().unwrap().is_empty());
        }
        let (_,replay)=wire::prepare(protocol,json!({"model":"fixture","messages":[{"role":"assistant","content":"Hello","cryzo_native":result.native,"tool_calls":[{"id":"call_1","function":{"name":"read","arguments":"{\"path\":\"a.txt\"}"}}]},{"role":"tool","tool_call_id":"call_1","content":"file contents"}]})).unwrap();
        if protocol != "chat-completions" {
            assert!(replay.to_string().contains("fixture-signature"));
        }
        server.abort();
    }
}

#[tokio::test]
async fn provider_errors_never_echo_secret_bodies() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            (
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "12")],
                "fixture-secret must never be returned",
            )
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (send, _) = mpsc::channel(1);
    let error = provider::completion_events(
        &endpoint,
        "fixture-secret",
        json!({"messages":[],"model":"fixture"}),
        send,
        CancellationToken::new(),
    )
    .await
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("429") && error.contains("12"));
    assert!(!error.contains("fixture-secret"));
    server.abort();
}
