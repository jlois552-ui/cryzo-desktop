use axum::{
    Json, Router,
    body::Body,
    http::{Request, StatusCode},
    routing::post,
};
use cryzo_engine::{
    catalog::{Api, Catalog, Model, Provider},
    native_sessions::ResolvedProvider,
    service::{Engine, router},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;

async fn call(app: &Router, method: &str, path: &str, input: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", "Bearer native-test")
                .header("content-type", "application/json")
                .body(if input.is_null() {
                    Body::empty()
                } else {
                    Body::from(input.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}
async fn fixture() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let app=Router::new().route("/v1/chat/completions",post(move|Json(body):Json<Value>|{let counter=counter.clone();async move {
        assert_eq!(body["model"],"fixture-model");
        let step=counter.fetch_add(1,Ordering::SeqCst);
        let event=if step==0{json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"write-1","function":{"name":"write","arguments":json!({"path":"result.txt","content":"Native agent wrote this."}).to_string()}}]},"finish_reason":"tool_calls"}]})}
            else {assert_eq!(body["messages"].as_array().unwrap().last().unwrap()["role"],"tool");json!({"choices":[{"delta":{"content":"Saved the file."},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":4}})};
        ([("content-type","text/event-stream")],format!("data: {event}\n\ndata: [DONE]\n\n"))
    }}));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, calls, server)
}
fn engine(path: &std::path::Path, url: &str) -> Arc<Engine> {
    let mut engine = Engine::open(&path.join("native.sqlite")).unwrap();
    engine.catalog = Catalog {
        upstream_commit: "fixture".into(),
        providers: vec![Provider {
            id: "fixture".into(),
            name: "Fixture".into(),
            credential_env: vec![],
            models: vec![Model {
                id: "fixture-model".into(),
                name: "Fixture".into(),
                apis: vec![Api {
                    protocol: "chat-completions".into(),
                    base_url: Some(url.into()),
                    compatibility: json!({}),
                }],
                details: json!({"capabilities":{"tools":{"supported":true}}}),
            }],
        }],
    };
    engine.store.desktop_put_meta("credentials","cred_fixture",Some(&json!({"id":"cred_fixture","integrationID":"fixture","label":"Fixture reference","active":true,"value":{"type":"external","methodID":"native-keyring"}}))).unwrap();
    Arc::new(engine)
}
async fn admitted(app: &Router, path: &std::path::Path) -> String {
    let (status,created)=call(app,"POST","/api/session",json!({"location":{"directory":path.to_string_lossy()},"model":{"providerID":"fixture","id":"fixture-model"}})).await;
    assert_eq!(status, 200);
    let id = created["data"]["id"].as_str().unwrap().to_owned();
    let (status, _) = call(
        app,
        "POST",
        &format!("/api/session/{id}/prompt"),
        json!({"id":"msg_fixture","text":"Write result.txt\n\nHidden context","metadata":{"displayText":"Write result.txt"},"resume":false}),
    )
    .await;
    assert_eq!(status, 200);
    let (status, retried) = call(
        app,
        "POST",
        &format!("/api/session/{id}/prompt"),
        json!({"id":"msg_fixture","text":"Do something different","resume":false}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        retried["data"]["payload"]["text"],
        "Write result.txt\n\nHidden context"
    );
    assert_eq!(
        retried["data"]["payload"]["metadata"]["displayText"],
        "Write result.txt"
    );
    id
}
async fn pending(app: &Router, id: &str) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let (_, value) = call(
                app,
                "GET",
                &format!("/api/session/{id}/permission"),
                Value::Null,
            )
            .await;
            if let Some(permission) = value["data"].as_array().and_then(|items| items.first()) {
                return permission["id"].as_str().unwrap().to_owned();
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap()
}
async fn idle(app: &Router) {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let (_, value) = call(app, "GET", "/api/session/active", Value::Null).await;
            if value["data"].as_object().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn real_agent_waits_for_exact_approval_writes_and_restores_history() {
    let (url, calls, server) = fixture().await;
    let temp = tempfile::tempdir().unwrap();
    let engine = engine(temp.path(), &url);
    let app = router(engine.clone(), "native-test".into());
    let id = admitted(&app, temp.path()).await;
    engine
        .clone()
        .start_session(
            id.clone(),
            ResolvedProvider {
                endpoint: url.clone(),
                secret: "fixture-secret".into(),
                model: json!({"providerID":"fixture","id":"fixture-model"}),
            },
        )
        .await
        .unwrap();
    let request = pending(&app, &id).await;
    assert!(!temp.path().join("result.txt").exists());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/session/ses_other/permission/{request}/reply"),
        json!({"decision":"once"}),
    )
    .await;
    assert_eq!(status, 400);
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/session/{id}/permission/{request}/reply"),
        json!({"decision":"once"}),
    )
    .await;
    assert_eq!(status, 204);
    idle(&app).await;
    assert_eq!(
        std::fs::read_to_string(temp.path().join("result.txt")).unwrap(),
        "Native agent wrote this."
    );
    let (_, messages) = call(
        &app,
        "GET",
        &format!("/api/session/{id}/message"),
        Value::Null,
    )
    .await;
    assert_eq!(messages["data"].as_array().unwrap().len(), 4);
    assert_eq!(messages["data"][2]["content"][0]["text"], "Saved the file.");
    assert_eq!(messages["data"][3]["type"], "idle");
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/session/{id}/prompt"),
        json!({"id":"msg_fixture","text":"Retried after completion"}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    if let Some(path) = std::env::var_os("CRYZO_CONTRACT_CAPTURE") {
        let mut capture = json!({"session":engine.store.desktop_state(&id).unwrap()["info"],"messages":messages["data"],"events":engine.store.desktop_log(&id,0).unwrap()});
        for resource in ["agent", "model", "provider", "integration"] {
            let (_, response) = call(&app, "GET", &format!("/api/{resource}"), Value::Null).await;
            capture[resource] = response["data"].clone();
        }
        std::fs::write(path, capture.to_string()).unwrap();
    }
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/session/{id}/permission/{request}/reply"),
        json!({"decision":"once"}),
    )
    .await;
    assert_eq!(status, 400);
    drop(app);
    drop(engine);
    let recovered = Engine::open(&temp.path().join("native.sqlite")).unwrap();
    let state = recovered.store.desktop_state(&id).unwrap();
    assert_eq!(state["info"]["outcome"], "succeeded");
    assert_eq!(state["messages"], messages["data"]);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    if let Some(destination) = std::env::var_os("CRYZO_UI_FIXTURE") {
        drop(recovered);
        let directory = temp.keep();
        std::fs::copy(
            directory.join("native.sqlite"),
            directory.join("cryzo.sqlite"),
        )
        .unwrap();
        std::fs::write(destination, directory.to_string_lossy().as_bytes()).unwrap();
    }
    server.abort();
}
#[tokio::test]
async fn interrupt_cancels_pending_write_without_effects() {
    let (url, calls, server) = fixture().await;
    let temp = tempfile::tempdir().unwrap();
    let engine = engine(temp.path(), &url);
    let app = router(engine.clone(), "native-test".into());
    let id = admitted(&app, temp.path()).await;
    engine
        .clone()
        .start_session(
            id.clone(),
            ResolvedProvider {
                endpoint: url,
                secret: "fixture-secret".into(),
                model: json!({"providerID":"fixture","id":"fixture-model"}),
            },
        )
        .await
        .unwrap();
    let request = pending(&app, &id).await;
    let (status, response) = call(
        &app,
        "POST",
        &format!("/api/session/{id}/interrupt"),
        json!({}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(response["interrupted"], true);
    idle(&app).await;
    assert!(!temp.path().join("result.txt").exists());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        engine.store.desktop_state(&id).unwrap()["info"]["outcome"],
        "interrupted"
    );
    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/session/{id}/permission/{request}/reply"),
        json!({"decision":"once"}),
    )
    .await;
    assert_eq!(status, 400);
    server.abort();
}
#[tokio::test]
async fn native_pty_runs_a_process_and_tickets_cannot_be_reused() {
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(&temp.path().join("pty.sqlite")).unwrap();
    let input = if cfg!(windows) {
        json!({"command":"cmd.exe","args":["/D","/Q","/C","echo NATIVE_PTY_OK"]})
    } else {
        json!({"command":"/bin/sh","args":["-c","printf NATIVE_PTY_OK"]})
    };
    let info = engine.ptys.create(temp.path(), &input).unwrap();
    let id = info["id"].as_str().unwrap();
    let ticket = engine.ptys.ticket(id, temp.path()).unwrap();
    let pty = engine.ptys.connect(id, temp.path(), &ticket).unwrap();
    assert!(engine.ptys.connect(id, temp.path(), &ticket).is_err());
    let output = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let (chunks, _, _) = pty.attach(None).unwrap();
            let text = chunks
                .into_iter()
                .map(|chunk| chunk.text)
                .collect::<String>();
            if text.contains("NATIVE_PTY_OK") {
                break text;
            }
            let _ = pty.write(b"\x1b[1;1R");
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    assert!(output.contains("NATIVE_PTY_OK"));
    pty.update(&json!({"size":{"rows":32,"cols":100}})).unwrap();
    engine.ptys.remove(id, temp.path()).unwrap();
}
