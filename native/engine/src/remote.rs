//! Remote control: an outbound link from this engine to the Cryzo relay.
//!
//! A paired phone or browser sends the same HTTP requests the desktop renderer sends. Each one
//! runs against the local router with the local token, so approvals bound to exact operations,
//! credential handling and every other rule apply unchanged. The engine dials out, so the
//! computer never listens on a public port. Only the upstream `/api/` contract is reachable.
use crate::{credentials::Credentials, free_tier::gateway, service::Engine};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Router,
    body::Body,
    http::{HeaderName, HeaderValue, Method, Request, header},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

/// The OS credential store entry that holds this computer's host ID and secret.
pub const CREDENTIAL: &str = "cryzo-remote";
const CHUNK: usize = 256 * 1024;
const MAX_BODY: usize = 3_000_000;

#[derive(Clone, Serialize, Deserialize)]
pub struct HostCredential {
    pub host: String,
    pub secret: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub connected: bool,
    pub clients: u32,
    pub error: Option<String>,
}

/// A running link. Dropping it disconnects.
pub struct Link {
    cancel: CancellationToken,
    status: watch::Receiver<Status>,
}

impl Link {
    pub fn start(
        engine: Arc<Engine>,
        router: Router,
        token: String,
        credential: HostCredential,
    ) -> Self {
        let cancel = CancellationToken::new();
        let (status, receiver) = watch::channel(Status::default());
        tokio::spawn(run(
            engine,
            router,
            token,
            credential,
            status,
            cancel.clone(),
        ));
        Self {
            cancel,
            status: receiver,
        }
    }

    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn run(
    engine: Arc<Engine>,
    router: Router,
    token: String,
    credential: HostCredential,
    status: watch::Sender<Status>,
    cancel: CancellationToken,
) {
    let mut delay = Duration::from_secs(1);
    while !cancel.is_cancelled() {
        let started = std::time::Instant::now();
        let result = async {
            let (ticket, relay) = ticket(&credential).await?;
            let mut url = url::Url::parse(&relay).context("invalid relay URL")?;
            url.query_pairs_mut().append_pair("ticket", &ticket);
            let (socket, _) = tokio_tungstenite::connect_async(url.as_str())
                .await
                .context("could not reach the remote control relay")?;
            status.send_modify(|value| {
                value.connected = true;
                value.error = None;
            });
            serve(
                socket,
                engine.clone(),
                router.clone(),
                &token,
                &status,
                &cancel,
            )
            .await
        }
        .await;
        status.send_modify(|value| {
            value.connected = false;
            value.clients = 0;
            value.error = result.err().map(|error| format!("{error:#}"));
        });
        if started.elapsed() > Duration::from_secs(60) {
            delay = Duration::from_secs(1);
        }
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
        }
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

#[derive(Default)]
struct Tasks {
    requests: HashMap<(String, u64), CancellationToken>,
    sockets: HashMap<(String, u64), (Arc<crate::pty::Pty>, CancellationToken)>,
}

/// Serves one relay connection until it closes or `cancel` fires.
pub(crate) async fn serve<S>(
    socket: tokio_tungstenite::WebSocketStream<S>,
    engine: Arc<Engine>,
    router: Router,
    token: &str,
    status: &watch::Sender<Status>,
    cancel: &CancellationToken,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut sink, mut stream) = socket.split();
    let (send, mut outgoing) = mpsc::unbounded_channel::<Value>();
    let tasks = Arc::new(Mutex::new(Tasks::default()));
    let session = cancel.child_token();
    let result = loop {
        tokio::select! {
            _ = session.cancelled() => break Ok(()),
            frame = outgoing.recv() => {
                let Some(frame) = frame else { break Ok(()) };
                if let Err(error) = sink.send(Message::text(frame.to_string())).await {
                    break Err(error.into());
                }
            }
            message = stream.next() => {
                let text = match message {
                    None => break Ok(()),
                    Some(Err(error)) => break Err(error.into()),
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) => break Ok(()),
                    Some(Ok(_)) => continue,
                };
                let Ok(frame) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                handle(frame, &engine, &router, token, status, &send, &tasks, &session).await;
            }
        }
    };
    session.cancel();
    let mut tasks = tasks.lock().await;
    tasks.requests.drain().for_each(|(_, task)| task.cancel());
    tasks
        .sockets
        .drain()
        .for_each(|(_, (_, task))| task.cancel());
    let _ = sink.close().await;
    result
}

#[allow(clippy::too_many_arguments)]
async fn handle(
    frame: Value,
    engine: &Arc<Engine>,
    router: &Router,
    token: &str,
    status: &watch::Sender<Status>,
    send: &mpsc::UnboundedSender<Value>,
    tasks: &Arc<Mutex<Tasks>>,
    session: &CancellationToken,
) {
    let client = frame["c"].as_str().unwrap_or_default().to_owned();
    let id = frame["id"].as_u64().unwrap_or_default();
    match frame["t"].as_str().unwrap_or_default() {
        "clients" => status.send_modify(|value| {
            value.clients = frame["n"].as_u64().unwrap_or_default().min(u32::MAX as u64) as u32
        }),
        "gone" => {
            let mut tasks = tasks.lock().await;
            tasks.requests.retain(|(owner, _), task| {
                let keep = owner != &client;
                if !keep {
                    task.cancel();
                }
                keep
            });
            tasks.sockets.retain(|(owner, _), (_, task)| {
                let keep = owner != &client;
                if !keep {
                    task.cancel();
                }
                keep
            });
        }
        "req" if !client.is_empty() => {
            let task = session.child_token();
            tasks
                .lock()
                .await
                .requests
                .insert((client.clone(), id), task.clone());
            let (router, token, send, tasks) = (
                router.clone(),
                token.to_owned(),
                send.clone(),
                tasks.clone(),
            );
            tokio::spawn(async move {
                if let Err(error) = request(&frame, router, &token, &send, &task).await {
                    let _ = send.send(
                        json!({"t":"error","c":client,"id":id,"message":format!("{error:#}")}),
                    );
                }
                tasks.lock().await.requests.remove(&(client, id));
            });
        }
        "abort" => {
            if let Some(task) = tasks.lock().await.requests.remove(&(client, id)) {
                task.cancel();
            }
        }
        "ws" if !client.is_empty() => {
            let (engine, send, tasks, session) =
                (engine.clone(), send.clone(), tasks.clone(), session.clone());
            tokio::spawn(async move {
                let code = match terminal(&frame, &engine, &send, &tasks, &session).await {
                    Ok(()) => 1000,
                    Err(_) => 1011,
                };
                tasks.lock().await.sockets.remove(&(client.clone(), id));
                let _ = send.send(json!({"t":"ws-close","c":client,"id":id,"code":code}));
            });
        }
        "ws-send" => {
            let pty = tasks
                .lock()
                .await
                .sockets
                .get(&(client, id))
                .map(|(pty, _)| pty.clone());
            let bytes = frame["data"]
                .as_str()
                .and_then(|data| STANDARD.decode(data).ok());
            if let (Some(pty), Some(bytes)) = (pty, bytes) {
                let _ = tokio::task::spawn_blocking(move || pty.write(&bytes)).await;
            }
        }
        "ws-close" => {
            if let Some((_, task)) = tasks.lock().await.sockets.get(&(client, id)) {
                task.cancel();
            }
        }
        _ => {}
    }
}

async fn request(
    frame: &Value,
    router: Router,
    token: &str,
    send: &mpsc::UnboundedSender<Value>,
    cancel: &CancellationToken,
) -> Result<()> {
    let client = frame["c"].as_str().unwrap_or_default();
    let id = frame["id"].as_u64().unwrap_or_default();
    let path = frame["path"].as_str().context("missing path")?;
    ensure!(
        path.starts_with("/api/") && !path.contains("://") && !path.contains(".."),
        "remote clients can only use the session API"
    );
    let method = Method::from_bytes(frame["method"].as_str().unwrap_or("GET").as_bytes())?;
    let body = match frame["body"].as_str() {
        Some(data) if !data.is_empty() => STANDARD.decode(data)?,
        _ => Vec::new(),
    };
    ensure!(body.len() <= MAX_BODY, "request body is too large");
    let mut builder = Request::builder().method(method).uri(path);
    for pair in frame["headers"].as_array().into_iter().flatten() {
        let (Some(name), Some(value)) = (pair[0].as_str(), pair[1].as_str()) else {
            continue;
        };
        let name = HeaderName::from_bytes(name.as_bytes())?;
        // The remote client never supplies the local token or connection-level headers.
        if [
            header::AUTHORIZATION,
            header::HOST,
            header::COOKIE,
            header::CONNECTION,
            header::UPGRADE,
            header::CONTENT_LENGTH,
        ]
        .contains(&name)
        {
            continue;
        }
        builder = builder.header(name, HeaderValue::from_str(value)?);
    }
    let request = builder
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))?;
    let response = tokio::select! {
        _ = cancel.cancelled() => return Ok(()),
        response = router.oneshot(request) => response?,
    };
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| Some(json!([name.as_str(), value.to_str().ok()?])))
        .collect::<Vec<_>>();
    send.send(json!({"t":"head","c":client,"id":id,"status":response.status().as_u16(),"headers":headers}))?;
    let mut body = response.into_body().into_data_stream();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            chunk = body.next() => match chunk {
                None => break,
                Some(Err(error)) => bail!(error),
                Some(Ok(bytes)) => {
                    for part in bytes.chunks(CHUNK) {
                        send.send(json!({"t":"data","c":client,"id":id,"data":STANDARD.encode(part)}))?;
                    }
                }
            }
        }
    }
    send.send(json!({"t":"end","c":client,"id":id}))?;
    Ok(())
}

/// A terminal socket, with the same contract as the desktop's native sockets.
async fn terminal(
    frame: &Value,
    engine: &Arc<Engine>,
    send: &mpsc::UnboundedSender<Value>,
    tasks: &Arc<Mutex<Tasks>>,
    session: &CancellationToken,
) -> Result<()> {
    let client = frame["c"].as_str().unwrap_or_default().to_owned();
    let id = frame["id"].as_u64().unwrap_or_default();
    let url = url::Url::parse(&format!(
        "http://engine{}",
        frame["path"].as_str().context("missing path")?
    ))?;
    let parts: Vec<_> = url.path().split('/').collect();
    ensure!(
        parts.len() == 5 && parts[1] == "api" && parts[2] == "pty" && parts[4] == "connect",
        "unsupported socket endpoint"
    );
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let directory = std::path::PathBuf::from(
        query
            .get("location[directory]")
            .context("missing terminal location")?,
    );
    let pty = engine.ptys.connect(
        parts[3],
        &directory,
        query.get("ticket").context("missing terminal ticket")?,
    )?;
    let cursor = query
        .get("cursor")
        .map(|value| value.parse::<u64>())
        .transpose()?;
    let (chunks, watermark, mut output) = pty.attach(cursor)?;
    let cancel = session.child_token();
    tasks
        .lock()
        .await
        .sockets
        .insert((client.clone(), id), (pty.clone(), cancel.clone()));
    send.send(json!({"t":"ws-open","c":client,"id":id}))?;
    for chunk in chunks {
        send.send(json!({"t":"ws-data","c":client,"id":id,"data":chunk.text}))?;
    }
    send.send(json!({"t":"ws-cursor","c":client,"id":id,"cursor":watermark}))?;
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            next = output.recv() => match next {
                Ok(chunk) if chunk.start >= watermark => {
                    send.send(json!({"t":"ws-data","c":client,"id":id,"data":chunk.text}))?;
                }
                Ok(_) => {}
                Err(_) => return Ok(()),
            },
            _ = tick.tick() => if pty.drained() && output.is_empty() { return Ok(()) },
        }
    }
}

/// A short-lived relay ticket and the relay's address, from the gateway.
async fn ticket(credential: &HostCredential) -> Result<(String, String)> {
    let data = call(
        reqwest::Method::POST,
        "remote/ticket",
        None,
        json!({"role":"host","id":credential.host,"secret":credential.secret}),
    )
    .await?;
    Ok((
        data["ticket"]
            .as_str()
            .context("gateway sent no ticket")?
            .to_owned(),
        data["relay"]
            .as_str()
            .context("gateway sent no relay")?
            .to_owned(),
    ))
}

/// This computer's saved host credential, if remote control was turned on.
pub fn saved() -> Option<HostCredential> {
    Credentials::get(CREDENTIAL)
        .ok()
        .and_then(|value| serde_json::from_str(&value).ok())
}

/// Registers this computer with the user's Cryzo account and saves its secret in the OS store.
pub async fn enable(name: &str) -> Result<HostCredential> {
    if let Some(saved) = saved() {
        return Ok(saved);
    }
    let data = call(
        reqwest::Method::POST,
        "remote/host",
        Some(account()?),
        json!({"name":name}),
    )
    .await?;
    let credential: HostCredential = serde_json::from_value(data)?;
    Credentials::set(CREDENTIAL, &serde_json::to_string(&credential)?)?;
    Ok(credential)
}

/// Forgets this computer: every paired device loses access.
pub async fn disable(credential: &HostCredential) -> Result<()> {
    let removed = call(
        reqwest::Method::POST,
        "remote/host/remove",
        Some(account()?),
        json!({"host":credential.host}),
    )
    .await;
    // A host the gateway no longer knows is already gone; forget it locally either way.
    if let Err(error) = &removed
        && !format!("{error:#}").contains("not set up")
    {
        return removed.map(|_| ());
    }
    Credentials::remove(CREDENTIAL)
}

/// A fresh pairing code (and link, when the web client is deployed) for a new device.
pub async fn pair(credential: &HostCredential) -> Result<Value> {
    call(
        reqwest::Method::POST,
        "remote/pair",
        Some(account()?),
        json!({"host":credential.host}),
    )
    .await
}

pub async fn clients(credential: &HostCredential) -> Result<Value> {
    call(
        reqwest::Method::GET,
        &format!("remote/clients?host={}", credential.host),
        Some(account()?),
        Value::Null,
    )
    .await
}

pub async fn revoke(client: &str) -> Result<()> {
    call(
        reqwest::Method::POST,
        "remote/revoke",
        Some(account()?),
        json!({"client":client}),
    )
    .await
    .map(|_| ())
}

pub(crate) fn account() -> Result<String> {
    Credentials::get("cryzo").context("Sign in to Cryzo first.")
}

/// A call to the Convex gateway; returns its `data` or its error message.
pub(crate) async fn call(
    method: reqwest::Method,
    path: &str,
    bearer: Option<String>,
    body: Value,
) -> Result<Value> {
    // Publishing deploys an app before the gateway answers, which can take most of a minute.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut request = client.request(method.clone(), format!("{}/{path}", gateway()?));
    if let Some(bearer) = bearer {
        request = request.bearer_auth(bearer);
    }
    if method != reqwest::Method::GET {
        request = request.json(&body);
    }
    let response = request.send().await?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        bail!(
            "{}",
            value["error"]
                .as_str()
                .unwrap_or("The Cryzo cloud refused the request.")
        );
    }
    Ok(value["data"].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::Request as AxumRequest,
        http::StatusCode,
        middleware::{self, Next},
        response::Response,
        routing::get,
    };
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, connect_async};

    fn engine() -> (tempfile::TempDir, Arc<Engine>) {
        let directory = tempfile::tempdir().unwrap();
        let engine = Arc::new(Engine::open(&directory.path().join("cryzo.sqlite")).unwrap());
        (directory, engine)
    }

    // Stands in for the engine router: it requires the local token, like service::router.
    fn router() -> Router {
        async fn auth(request: AxumRequest, next: Next) -> Response {
            if request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                != Some("Bearer local")
            {
                return Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(Body::empty())
                    .unwrap();
            }
            next.run(request).await
        }
        Router::new()
            .route("/api/hello", get(|| async { "hi" }))
            .route(
                "/api/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    "late"
                }),
            )
            .route("/v0/health", get(|| async { "internal" }))
            .layer(middleware::from_fn(auth))
    }

    /// A relay stand-in: the test plays the relay side of one socket.
    async fn link() -> (
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        watch::Receiver<Status>,
        CancellationToken,
        tempfile::TempDir,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (directory, engine) = engine();
        let (status, receiver) = watch::channel(Status::default());
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            let (socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
            let _ = serve(socket, engine, router(), "local", &status, &stop).await;
        });
        let (stream, _) = listener.accept().await.unwrap();
        (
            accept_async(stream).await.unwrap(),
            receiver,
            cancel,
            directory,
        )
    }

    async fn next(relay: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> Value {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), relay.next())
                .await
                .expect("frame in time")
            {
                Some(Ok(Message::Text(text))) => {
                    return serde_json::from_str(text.as_str()).unwrap();
                }
                Some(Ok(_)) => continue,
                other => panic!("relay socket ended: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn requests_run_with_the_local_token_and_stream_back() {
        let (mut relay, _status, cancel, _directory) = link().await;
        relay
            .send(Message::text(
                json!({"t":"req","c":"phone","id":7,"method":"GET","path":"/api/hello",
                    "headers":[["authorization","Bearer stolen"]]})
                .to_string(),
            ))
            .await
            .unwrap();
        let head = next(&mut relay).await;
        assert_eq!(head["t"], "head");
        assert_eq!(head["c"], "phone");
        assert_eq!(head["id"], 7);
        assert_eq!(head["status"], 200);
        let data = next(&mut relay).await;
        assert_eq!(
            STANDARD.decode(data["data"].as_str().unwrap()).unwrap(),
            b"hi"
        );
        assert_eq!(next(&mut relay).await["t"], "end");
        cancel.cancel();
    }

    #[tokio::test]
    async fn only_the_session_api_is_reachable() {
        let (mut relay, _status, cancel, _directory) = link().await;
        for path in ["/v0/health", "/api/../v0/health", "http://evil/api/hello"] {
            relay
                .send(Message::text(
                    json!({"t":"req","c":"phone","id":1,"method":"GET","path":path}).to_string(),
                ))
                .await
                .unwrap();
            let frame = next(&mut relay).await;
            assert_eq!(frame["t"], "error", "{path}");
        }
        cancel.cancel();
    }

    #[tokio::test]
    async fn client_counts_are_reported_and_a_gone_client_loses_its_requests() {
        let (mut relay, mut status, cancel, _directory) = link().await;
        relay
            .send(Message::text(json!({"t":"clients","n":2}).to_string()))
            .await
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(5),
            status.wait_for(|value| value.clients == 2),
        )
        .await
        .unwrap()
        .unwrap();
        relay
            .send(Message::text(
                json!({"t":"req","c":"phone","id":3,"method":"GET","path":"/api/slow"}).to_string(),
            ))
            .await
            .unwrap();
        relay
            .send(Message::text(json!({"t":"gone","c":"phone"}).to_string()))
            .await
            .unwrap();
        // The cancelled request never answers; the next request still does.
        relay
            .send(Message::text(
                json!({"t":"req","c":"laptop","id":4,"method":"GET","path":"/api/hello"})
                    .to_string(),
            ))
            .await
            .unwrap();
        let frame = next(&mut relay).await;
        assert_eq!(
            (frame["c"].as_str(), frame["id"].as_u64()),
            (Some("laptop"), Some(4))
        );
        cancel.cancel();
    }
}
