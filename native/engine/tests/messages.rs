use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn get(app: &axum::Router, uri: &str) -> (u16, Value) {
    let request = axum::http::Request::builder()
        .uri(uri)
        .header("authorization", "Bearer fixture-key")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn ids(page: &Value) -> Vec<&str> {
    page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["id"].as_str().unwrap())
        .collect()
}

// The desktop reopens a session with order=desc and a page limit, then reverses the page.
#[tokio::test]
async fn message_pages_follow_the_requested_order_across_cursors() {
    let temp = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(
        cryzo_engine::service::Engine::open(&temp.path().join("messages.sqlite")).unwrap(),
    );
    let app = cryzo_engine::service::router(engine.clone(), "fixture-key".into());
    let create = axum::http::Request::builder()
        .method("POST")
        .uri("/api/session")
        .header("authorization", "Bearer fixture-key")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({"id":"ses_pages","location":{"directory":temp.path()}}).to_string(),
        ))
        .unwrap();
    assert!(
        app.clone()
            .oneshot(create)
            .await
            .unwrap()
            .status()
            .is_success()
    );
    for id in ["msg_a", "msg_b", "msg_c"] {
        let item = json!({"type":"user","payload":{"text":id},"delivery":"steer"});
        engine
            .store
            .desktop_append(
                "ses_pages",
                "session.inbox.enqueued",
                json!({"inboxID":id,"item":item}),
            )
            .unwrap();
        engine
            .store
            .desktop_append(
                "ses_pages",
                "session.inbox.delivered",
                json!({"inboxID":id}),
            )
            .unwrap();
    }

    let (_, first) = get(&app, "/api/session/ses_pages/message?limit=2&order=desc").await;
    assert_eq!(ids(&first), ["msg_c", "msg_b"]);
    let next = first["cursor"]["next"].as_str().unwrap();
    let (_, second) = get(
        &app,
        &format!("/api/session/ses_pages/message?limit=2&cursor={next}"),
    )
    .await;
    assert_eq!(ids(&second), ["msg_a"]);
    assert!(second["cursor"]["next"].is_null());

    let (_, ascending) = get(&app, "/api/session/ses_pages/message").await;
    assert_eq!(ids(&ascending), ["msg_a", "msg_b", "msg_c"]);
    let (status, invalid) = get(&app, "/api/session/ses_pages/message?cursor=bogus").await;
    assert_eq!(
        (status, &invalid["_tag"]),
        (400, &json!("InvalidCursorError"))
    );
}
