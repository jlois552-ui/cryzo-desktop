use anyhow::{Context, Result, bail};
use cryzo_engine::{
    credentials::Credentials,
    service::{Engine, router},
};
use std::{io::Read, path::PathBuf, sync::Arc};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("credential") {
        let provider = args
            .get(1)
            .context("usage: cryzo-engine credential <provider>; pass secret on stdin")?;
        let mut secret = String::new();
        std::io::stdin().read_to_string(&mut secret)?;
        Credentials::set(provider, secret.trim())?;
        eprintln!("Credential stored in the operating system credential store.");
        return Ok(());
    }
    if args.first().map(String::as_str) != Some("serve") {
        bail!("usage: cryzo-engine serve <data-directory> | credential <provider>");
    }
    let directory = PathBuf::from(args.get(1).context("data directory is required")?);
    std::fs::create_dir_all(&directory)?;
    let engine = Arc::new(Engine::open(&directory.join("cryzo.sqlite"))?);
    // Bind only loopback on an ephemeral port. Never expose local filesystem APIs on the LAN.
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let token = Uuid::new_v4().to_string();
    // This single machine-readable line is for the owning desktop process, not a log file.
    println!(
        "{}",
        serde_json::json!({"url":format!("http://{}",listener.local_addr()?),"token":token})
    );
    axum::serve(listener, router(engine, token))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
