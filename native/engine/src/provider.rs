use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use url::Url;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// API-key Chat Completions transport shared by compatible providers.
/// OAuth, Bedrock, Responses, Anthropic and Gemini need their own authenticated adapters.
pub async fn chat(
    endpoint: &str,
    secret: &str,
    model: &str,
    messages: &[Message],
    output: mpsc::Sender<String>,
    cancel: CancellationToken,
) -> Result<String> {
    let (send, mut receive) = mpsc::channel(64);
    let relay = tokio::spawn(async move {
        while let Some(text) = receive.recv().await {
            let _ = output.send(text).await;
        }
    });
    let result = completion(
        endpoint,
        secret,
        json!({"model":model,"messages":messages}),
        send,
        cancel,
    )
    .await;
    let _ = relay.await;
    result.map(|result| result.text)
}

#[derive(Default, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}
pub struct Completion {
    pub text: String,
    pub reasoning: String,
    pub native: Value,
    pub calls: Vec<ToolCall>,
    pub usage: Value,
    pub finish: String,
}
#[derive(Debug)]
pub enum Delta {
    Text(String),
    Reasoning(String),
}
pub async fn completion(
    endpoint: &str,
    secret: &str,
    request: Value,
    output: mpsc::Sender<String>,
    cancel: CancellationToken,
) -> Result<Completion> {
    let (send, mut receive) = mpsc::channel(64);
    let relay = tokio::spawn(async move {
        while let Some(delta) = receive.recv().await {
            if let Delta::Text(text) = delta {
                let _ = output.send(text).await;
            }
        }
    });
    let result = completion_events(endpoint, secret, request, send, cancel).await;
    let _ = relay.await;
    result
}

pub async fn completion_events(
    endpoint: &str,
    secret: &str,
    mut request: Value,
    output: mpsc::Sender<Delta>,
    cancel: CancellationToken,
) -> Result<Completion> {
    let mut url = Url::parse(endpoint)?;
    ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"))),
        "provider URL must use HTTPS (except local inference)"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "credentials must not appear in provider URLs"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "provider URL cannot contain query parameters or fragments"
    );
    let gateway = request
        .as_object_mut()
        .context("invalid request")?
        .remove("cryzo_gateway")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let protocol = request
        .as_object_mut()
        .context("invalid request")?
        .remove("cryzo_protocol")
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "chat-completions".into());
    let headers = request
        .as_object_mut()
        .context("invalid request")?
        .remove("cryzo_headers");
    let (path, mut request) = crate::wire::prepare(&protocol, request)?;
    if protocol == "chat-completions"
        && let Some(messages) = request["messages"].as_array_mut()
    {
        for message in messages {
            if let Some(details) = message
                .pointer("/cryzo_native/chat-completions")
                .and_then(Value::as_array)
                .filter(|v| !v.is_empty())
            {
                message["reasoning_details"] = json!(details);
            }
            message
                .as_object_mut()
                .context("invalid message")?
                .remove("cryzo_native");
        }
    }
    url.set_path(&format!("{}/{path}", url.path().trim_end_matches('/')));
    if protocol == "generate-content" {
        url.set_query(Some("alt=sse"));
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(600))
        .build()?;
    let mut builder = client.post(url);
    builder = match protocol.as_str() {
        "messages" => builder
            .header("x-api-key", secret)
            .header("anthropic-version", "2023-06-01"),
        "generate-content" => builder.header("x-goog-api-key", secret),
        _ => builder.bearer_auth(secret),
    };
    for (name, value) in headers
        .as_ref()
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        if let Some(value) = value.as_str() {
            builder = builder.header(name, value);
        }
    }
    let response = tokio::select! {
        _ = cancel.cancelled() => bail!("generation interrupted"),
        response = builder.json(&request).send() => response.map_err(|_|anyhow::anyhow!("could not connect to provider"))?,
    };
    // Do not include arbitrary provider bodies: some services echo credentials in diagnostics.
    if !response.status().is_success() {
        let status = response.status();
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if gateway {
            let data = response.json::<Value>().await.unwrap_or(Value::Null);
            if let Some(message) = data["error"].as_str().filter(|message| {
                [
                    "daily limit reached; resets at 00:00 UTC",
                    "one generation at a time",
                    "free service busy; retry shortly",
                    "duplicate step",
                    "Sign in to Cryzo Free",
                    "This free model is temporarily unavailable",
                    "Free inference is not configured",
                    "NVIDIA is busy; retry shortly",
                    "Could not connect to NVIDIA",
                    "NVIDIA inference failed",
                    "Cryzo Free is temporarily unavailable",
                ]
                .contains(message)
            }) {
                bail!("{message}")
            }
        }
        bail!(
            "provider returned HTTP {}{}",
            status,
            retry
                .map(|v| format!("; retry after {v} seconds"))
                .unwrap_or_default()
        );
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    // ChatGPT sign-in streams arrive without a content type; reject only a different declared format.
    if !content_type.is_empty() && !content_type.starts_with("text/event-stream") {
        let status = response.status();
        // Name only the provider's error code or the content type, never the body.
        let code = response.json::<Value>().await.ok().and_then(|data| {
            data.pointer("/error/code")
                .or(data.pointer("/error/type"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        bail!(
            "provider did not return an event stream (HTTP {}, {})",
            status.as_u16(),
            code.unwrap_or(content_type)
        );
    }
    let mut stream = response.bytes_stream();
    let mut decoder = Sse::default();
    let mut wire = crate::wire::Decoder::default();
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut finished = false;
    let mut calls = std::collections::BTreeMap::<u64, ToolCall>::new();
    let mut usage = json!({});
    let mut finish = "stop".to_owned();
    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => bail!("generation interrupted"),
            next = stream.next() => next,
        };
        let Some(chunk) = next else { break };
        for data in decoder.push(&chunk?)? {
            if data == "[DONE]" {
                finished = true;
                continue;
            }
            let value = wire.event(
                &protocol,
                serde_json::from_str(&data).context("invalid provider event")?,
            )?;
            if value["usage"].is_object() {
                usage = value["usage"].clone();
            }
            ensure!(
                value.get("error").is_none(),
                "provider reported a generation error"
            );
            if let Some(delta) = value
                .pointer("/choices/0/delta/reasoning_content")
                .or_else(|| value.pointer("/choices/0/delta/reasoning"))
                .and_then(Value::as_str)
            {
                reasoning.push_str(delta);
                ensure!(
                    reasoning.len() <= 4_000_000,
                    "provider reasoning is too large"
                );
                tokio::select! { _ = cancel.cancelled() => bail!("generation interrupted"), _ = output.send(Delta::Reasoning(delta.to_owned())) => {} }
            }
            if let Some(delta) = value
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
            {
                text.push_str(delta);
                ensure!(text.len() <= 4_000_000, "provider text is too large");
                tokio::select! { _ = cancel.cancelled() => bail!("generation interrupted"), _ = output.send(Delta::Text(delta.to_owned())) => {} }
            }
            if let Some(deltas) = value
                .pointer("/choices/0/delta/tool_calls")
                .and_then(Value::as_array)
            {
                for delta in deltas {
                    let index = delta["index"].as_u64().context("tool call index missing")?;
                    ensure!(index < 64, "too many tool calls");
                    let call = calls.entry(index).or_default();
                    if let Some(id) = delta["id"].as_str() {
                        call.id.push_str(id);
                    }
                    if let Some(name) = delta.pointer("/function/name").and_then(Value::as_str) {
                        call.name.push_str(name);
                    }
                    if let Some(args) = delta.pointer("/function/arguments").and_then(Value::as_str)
                    {
                        call.arguments.push_str(args);
                    }
                    ensure!(
                        call.arguments.len() <= 4_000_000
                            && call.name.len() <= 256
                            && call.id.len() <= 256,
                        "tool call exceeds limits"
                    );
                }
            }
            if value
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
                .is_some()
            {
                finished = true;
                finish = value
                    .pointer("/choices/0/finish_reason")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_owned();
            }
        }
    }
    ensure!(finished, "provider stream ended before completion");
    let calls = calls.into_values().collect::<Vec<_>>();
    ensure!(
        calls
            .iter()
            .all(|call| !call.id.is_empty() && !call.name.is_empty()),
        "provider returned an incomplete tool call"
    );
    Ok(Completion {
        text,
        reasoning,
        native: wire.native(&protocol),
        calls,
        usage,
        finish,
    })
}

/// Byte framing preserves Unicode when a codepoint is split between network chunks.
#[derive(Default)]
pub struct Sse {
    pending: Vec<u8>,
    data: Vec<String>,
}

impl Sse {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        self.pending.extend_from_slice(bytes);
        ensure!(
            self.pending.len() <= 8_000_000,
            "provider event exceeds size limit"
        );
        let mut result = Vec::new();
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = std::str::from_utf8(&line)?.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    result.push(self.data.join("\n"));
                    self.data.clear();
                }
                continue;
            }
            if let Some(data) = line.strip_prefix("data:") {
                self.data
                    .push(data.strip_prefix(' ').unwrap_or(data).to_owned());
            }
        }
        Ok(result)
    }
}
