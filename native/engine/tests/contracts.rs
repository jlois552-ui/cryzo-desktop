use cryzo_engine::{
    provider::Sse,
    service::{Engine, router},
    store::Store,
    tools::{Operation, Workspace},
};
use http_body_util::BodyExt;
use serde_json::json;
use std::{path::Path, sync::Arc};
use tower::ServiceExt;

#[tokio::test]
async fn desktop_location_registers_a_durable_project_and_requires_authentication() {
    use axum::{body::Body, http::Request};
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("sessions.sqlite");
    let engine = Arc::new(Engine::open(&database).unwrap());
    let app = router(engine, "private-native-token".into());
    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/info")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401);
    // Global settings use the unchanged desktop client's empty Location ref.
    // It must resolve to home, rather than attempting to canonicalize "".
    for endpoint in ["location", "integration", "provider", "model"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/{endpoint}?location%5Bdirectory%5D="))
                    .header("authorization", "Bearer private-native-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "global {endpoint} settings failed");
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        if endpoint == "location" {
            assert_eq!(body["project"]["id"], "global");
        }
        if endpoint == "integration" {
            for provider in ["nvidia", "openrouter", "openai", "anthropic", "google"] {
                assert!(
                    body["data"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|row| row["id"] == provider),
                    "missing BYOK {provider}"
                );
            }
        }
    }
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("location[directory]", directory.path().to_str().unwrap())
        .finish();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/location?{query}"))
                .header("authorization", "Bearer private-native-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let location: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(
        location["project"]["id"]
            .as_str()
            .unwrap()
            .starts_with("prj_")
    );
    assert_eq!(location["directory"], location["project"]["canonical"]);
    let project_id = location["project"]["id"].clone();
    drop(app);
    let engine = Arc::new(Engine::open(&database).unwrap());
    let app = router(engine, "new-native-token".into());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/project")
                .header("authorization", "Bearer new-native-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let projects: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_eq!(projects[0]["id"], project_id);
    assert!(projects[0]["time"]["created"].as_u64().unwrap() > 0);
}

#[test]
fn durable_admission_is_idempotent_across_restarts_and_session_scoped() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("sessions.sqlite");
    let store = Store::open(&database).unwrap();
    let first = store.create_session(directory.path(), "First").unwrap();
    let second = store.create_session(directory.path(), "Second").unwrap();
    assert!(
        store
            .admit(&first.id, "message-1", &json!({"content":"original"}))
            .unwrap()
    );
    assert!(
        !store
            .admit(&first.id, "message-1", &json!({"content":"retry"}))
            .unwrap()
    );
    assert!(
        store
            .admit(&second.id, "message-1", &json!({"content":"wrong session"}))
            .is_err()
    );
    assert_eq!(
        store.deliver(&first.id, "message-1").unwrap(),
        Some(json!({"content":"original"}))
    );
    assert!(store.deliver(&first.id, "message-1").unwrap().is_none());
    store.claim(&first.id).unwrap();
    assert!(store.claim(&first.id).is_err());
    drop(store);
    let store = Store::open(&database).unwrap();
    assert_eq!(store.session(&first.id).unwrap().status, "interrupted");
    assert!(
        !store
            .admit(&first.id, "message-1", &json!({"content":"replay"}))
            .unwrap()
    );
    assert_eq!(store.events(&first.id, 0).unwrap().len(), 2);
}

#[test]
fn a_second_engine_cannot_reset_the_running_owners_claims() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sessions.sqlite");
    let engine = Engine::open(&path).unwrap();
    let session = engine
        .store
        .create_session(directory.path(), "Owner")
        .unwrap();
    engine.store.claim(&session.id).unwrap();
    assert!(Engine::open(&path).is_err());
    assert_eq!(engine.store.session(&session.id).unwrap().status, "running");
    drop(engine);
    let recovered = Engine::open(&path).unwrap();
    assert_eq!(
        recovered.store.session(&session.id).unwrap().status,
        "interrupted"
    );
}

#[test]
fn approval_is_bound_to_operation_session_and_one_execution() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("sessions.sqlite")).unwrap();
    let first = store.create_session(directory.path(), "First").unwrap();
    let second = store.create_session(directory.path(), "Second").unwrap();
    let operation = json!({"tool":"shell","program":"git","args":["status"]});
    let approval = store.request_approval(&first.id, &operation).unwrap();
    assert!(
        store
            .consume_approval(&first.id, &approval, &operation)
            .is_err()
    );
    assert!(store.decide(&second.id, &approval, true).is_err());
    store.decide(&first.id, &approval, true).unwrap();
    assert!(
        store
            .consume_approval(
                &first.id,
                &approval,
                &json!({"tool":"shell","program":"git","args":["clean","-fd"]})
            )
            .is_err()
    );
    assert!(
        store
            .consume_approval(&second.id, &approval, &operation)
            .is_err()
    );
    store
        .consume_approval(&first.id, &approval, &operation)
        .unwrap();
    assert!(
        store
            .consume_approval(&first.id, &approval, &operation)
            .is_err()
    );
}

#[tokio::test]
async fn file_writes_require_approval_and_reject_workspace_escape() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("sessions.sqlite")).unwrap();
    let session = store.create_session(directory.path(), "Files").unwrap();
    let workspace = Workspace::open(directory.path()).unwrap();
    let operation = Operation::Write {
        path: "hello.txt".into(),
        content: "hello".into(),
    };
    assert!(
        workspace
            .execute(&store, &session.id, &operation, None)
            .await
            .is_err()
    );
    assert!(!directory.path().join("hello.txt").exists());
    let approval = store
        .request_approval(&session.id, &serde_json::to_value(&operation).unwrap())
        .unwrap();
    store.decide(&session.id, &approval, true).unwrap();
    workspace
        .execute(&store, &session.id, &operation, Some(&approval))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(directory.path().join("hello.txt")).unwrap(),
        "hello"
    );
    assert!(workspace.resolve("../outside.txt").is_err());
    assert!(
        workspace
            .resolve(&directory.path().to_string_lossy())
            .is_err()
    );
    assert!(workspace.resolve("nested/../../outside.txt").is_err());
}

#[cfg(unix)]
#[test]
fn linked_ancestors_cannot_escape_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), directory.path().join("escape")).unwrap();
    let workspace = Workspace::open(directory.path()).unwrap();
    assert!(workspace.resolve("escape/new.txt").is_err());
}

#[test]
fn sse_handles_fragmented_unicode_crlf_comments_and_multiline_data() {
    let mut decoder = Sse::default();
    let bytes = ": keepalive\r\ndata: {\"text\":\"🦀\"}\r\n\r\ndata: first\ndata: second\n\ndata: [DONE]\n\n".as_bytes();
    let mut frames = Vec::new();
    for byte in bytes {
        frames.extend(decoder.push(&[*byte]).unwrap());
    }
    assert_eq!(frames, vec!["{\"text\":\"🦀\"}", "first\nsecond", "[DONE]"]);
}

#[tokio::test]
async fn local_http_requires_credential_and_persists_real_session_events() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Arc::new(Engine::open(&directory.path().join("sessions.sqlite")).unwrap());
    let session = engine
        .store
        .create_session(directory.path(), "HTTP")
        .unwrap();
    engine
        .store
        .append(&session.id, "fixture", &json!({"value":42}))
        .unwrap();
    let app = router(engine, "private-test-token".into());
    let unauthorized = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/v0/health")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401);
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/v0/session/{}/events?after=0", session.id))
                .header("authorization", "Bearer private-test-token")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(events[0]["data"]["value"], 42);
}

#[test]
fn full_frozen_catalog_is_available_without_network_or_credentials() {
    let catalog = cryzo_engine::catalog::Catalog::load().unwrap();
    assert_eq!(
        catalog.upstream_commit,
        "caebbcc0807e630629725f320f35a09aa4a427be"
    );
    assert_eq!(catalog.providers.len(), 230);
    assert_eq!(
        catalog
            .providers
            .iter()
            .map(|provider| provider.models.len())
            .sum::<usize>(),
        8787
    );
    assert!(
        catalog
            .providers
            .iter()
            .any(|provider| provider.id == "nvidia")
    );
    assert!(
        catalog
            .providers
            .iter()
            .any(|provider| provider.id == "openrouter")
    );
    assert!(Workspace::open(Path::new("missing-workspace")).is_err());
}

#[tokio::test]
async fn provider_transport_streams_real_http_and_observes_cancellation() {
    use axum::{Router, http::HeaderMap, routing::post};
    use cryzo_engine::provider::{Message, chat};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|headers: HeaderMap| async move {
            assert_eq!(
                headers.get("authorization").unwrap(),
                "Bearer fixture-credential"
            );
            (
                [("content-type", "text/event-stream")],
                concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"Hello 🦀\"}}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n"
                ),
            )
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (send, mut receive) = mpsc::channel(8);
    let messages = [Message {
        role: "user".into(),
        content: "Hello".into(),
    }];
    let result = chat(
        &format!("http://{address}/v1"),
        "fixture-credential",
        "fixture-model",
        &messages,
        send,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(result, "Hello 🦀");
    assert_eq!(receive.recv().await.as_deref(), Some("Hello 🦀"));
    let (send, _) = mpsc::channel(8);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(
        chat(
            &format!("http://{address}/v1"),
            "fixture-credential",
            "fixture-model",
            &messages,
            send,
            cancel
        )
        .await
        .is_err()
    );
    server.abort();
}
