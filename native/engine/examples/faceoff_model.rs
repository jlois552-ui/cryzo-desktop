//! Shared model driver for the desktop benchmark. The refresh token stays in the OS store.
use anyhow::{Context, Result, ensure};
use cryzo_engine::{chatgpt, provider, service::Engine};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{Value, json};
use std::{io::Write, path::Path};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let source_path = args
        .next()
        .context("pass the dedicated Cryzo profile database")?;
    let scratch_path = args.next().context("pass the benchmark database path")?;
    let source = Connection::open_with_flags(source_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let account = source
        .query_row(
            "SELECT data FROM desktop_meta WHERE namespace='credentials' AND
             json_extract(data,'$.integrationID')='openai' AND json_extract(data,'$.active')=1 AND
             json_extract(data,'$.value.methodID')='chatgpt-token-sharing'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .context("Sign in with ChatGPT in the dedicated Windows profile first")?;
    drop(source);
    let account: Value = serde_json::from_str(&account)?;
    let engine = Engine::open(Path::new(&scratch_path))?;
    let client = account["value"]["metadata"]["clientID"]
        .as_str()
        .context("The ChatGPT connection has no client ID")?;
    engine.store.desktop_put_meta(
        "credentials",
        "faceoff",
        Some(&json!({"integrationID":"openai","active":true,
            "value":{"methodID":chatgpt::METHOD,"metadata":{"clientID":client}}})),
    )?;
    let mut output = std::io::stdout().lock();
    writeln!(output, "{}", json!({"ready":true}))?;
    output.flush()?;
    for line in std::io::stdin().lines() {
        let message = serde_json::from_str(&line?)?;
        let reply = match complete(&engine, message).await {
            Ok(result) => json!({"result":result}),
            Err(error) => json!({"error":error.to_string()}),
        };
        writeln!(output, "{reply}")?;
        output.flush()?;
    }
    Ok(())
}

async fn complete(engine: &Engine, input: Value) -> Result<Value> {
    ensure!(
        input["messages"].is_array() && input["tools"].is_array(),
        "Invalid benchmark input"
    );
    let secret = chatgpt::access_token(engine).await?;
    let request = json!({"model":"gpt-6.1-sol","reasoning_effort":"high",
        "cryzo_protocol":"responses","cryzo_chatgpt":true,
        "messages":input["messages"],"tools":input["tools"]});
    let (send, mut receive) = tokio::sync::mpsc::channel(32);
    let drain = tokio::spawn(async move { while receive.recv().await.is_some() {} });
    let result = provider::completion_events(
        "https://api.openai.com/v1",
        &secret,
        request,
        send,
        CancellationToken::new(),
    )
    .await;
    drain.await?;
    let result = result?;
    let calls: Vec<Value> = result.calls.into_iter()
        .map(|call| json!({"id":call.id,"type":"function","function":{"name":call.name,"arguments":call.arguments}}))
        .collect();
    Ok(json!({"text":result.text,"native":result.native,"calls":calls,"usage":result.usage}))
}
