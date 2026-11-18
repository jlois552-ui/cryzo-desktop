//! Checks before an app goes live, as Cursor, Lovable and Replit run on every publish. They block
//! files that must never be served (env files, keys, credential files, source maps) and source
//! files that contain what looks like a secret. Problems go back to the agent to fix; nothing is
//! published until the checks pass.
//!
//! Publishing then sends the build output to Cloudflare: the gateway opens an upload for the
//! files Cloudflare doesn't already have, this uploads them straight to Cloudflare with that
//! short-lived token, and the gateway deploys the app at `<slug>.<app domain>`.
use crate::{remote, service::Engine};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    extract::{RawQuery, State},
    http::StatusCode,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, PartialEq, Serialize)]
pub struct Problem {
    pub path: String,
    pub reason: &'static str,
}

const SKIPPED: [&str; 4] = ["node_modules", ".git", ".cryzo", ".wrangler"];
const MAX_SCAN: u64 = 2 * 1024 * 1024;

/// Every problem in the folder that would be published.
pub fn check(root: &Path) -> Result<Vec<Problem>> {
    // Source maps only matter where they would be served; a Worker build keeps its own for
    // stack traces, and those never reach a visitor.
    let served = output(root).ok();
    let mut problems = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if kind.is_symlink() || SKIPPED.contains(&name.as_str()) {
                continue;
            }
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let path = entry
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            if name.to_lowercase().ends_with(".map")
                && !served
                    .as_ref()
                    .is_some_and(|folder| entry.path().starts_with(folder))
            {
                continue;
            }
            if let Some(reason) = forbidden(&name) {
                problems.push(Problem { path, reason });
                continue;
            }
            if entry.metadata()?.len() <= MAX_SCAN
                && let Ok(text) = std::fs::read_to_string(entry.path())
                && let Some(reason) = secret(&text)
            {
                problems.push(Problem { path, reason });
            }
        }
    }
    problems.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(problems)
}

/// Build folders, most specific first. A folder with `index.html` and no `package.json` is a
/// plain static site and is published as it is.
const OUTPUTS: [&str; 4] = ["dist/client", "dist", "build", "out"];
/// A server module next to the static files (the Cloudflare Pages convention). It becomes the
/// app's Worker code instead of being served.
const MODULE: &str = "_worker.js";

pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/api/cryzo/publish", post(publish))
        .route("/api/cryzo/apps", get(apps))
        .route("/api/cryzo/apps/unpublish", post(unpublish))
        .route("/api/cryzo/apps/secrets", post(secrets))
        .route("/api/cryzo/apps/secrets/set", post(set_secret))
        .route("/api/cryzo/apps/secrets/delete", post(delete_secret))
        .with_state(engine)
}

/// The names of a published app's secrets. Values are never read back.
async fn secrets(Json(input): Json<Value>) -> ApiResult {
    let names = gateway("apps/secrets", json!({"slug":input["slug"]})).await?;
    Ok(Json(json!({"secrets":names})))
}

/// Saves a secret straight to the app's host. The value passes through without being stored,
/// logged or shown to the agent.
async fn set_secret(Json(input): Json<Value>) -> ApiResult {
    let body = json!({"slug":input["slug"],"name":input["name"],"value":input["value"]});
    gateway("apps/secrets/set", body).await?;
    Ok(Json(json!({"ok":true})))
}

async fn delete_secret(Json(input): Json<Value>) -> ApiResult {
    let body = json!({"slug":input["slug"],"name":input["name"]});
    gateway("apps/secrets/delete", body).await?;
    Ok(Json(json!({"ok":true})))
}

async fn gateway(path: &str, body: Value) -> std::result::Result<Value, (StatusCode, Json<Value>)> {
    let account = remote::account().map_err(invalid)?;
    remote::call(reqwest::Method::POST, path, Some(account), body)
        .await
        .map_err(invalid)
}

type ApiResult = std::result::Result<Json<Value>, (StatusCode, Json<Value>)>;

fn invalid(error: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"_tag":"InvalidRequestError","message":format!("{error:#}")})),
    )
}

/// Runs the checks and, when they pass, publishes. Problems come back for the agent to fix.
async fn publish(
    State(_engine): State<Arc<Engine>>,
    RawQuery(query): RawQuery,
    Json(input): Json<Value>,
) -> ApiResult {
    let root = crate::desktop_api::directory(query.as_deref()).map_err(invalid)?;
    let problems = check(&root).map_err(invalid)?;
    if !problems.is_empty() {
        return Ok(Json(json!({"problems":problems})));
    }
    let slug = input["slug"].as_str().unwrap_or_default();
    let url = deploy(&root, slug).await.map_err(invalid)?;
    Ok(Json(json!({"problems":[],"url":url})))
}

async fn apps() -> ApiResult {
    let account = remote::account().map_err(invalid)?;
    let apps = remote::call(reqwest::Method::GET, "apps", Some(account), Value::Null)
        .await
        .map_err(invalid)?;
    Ok(Json(json!({"apps":apps})))
}

async fn unpublish(Json(input): Json<Value>) -> ApiResult {
    let account = remote::account().map_err(invalid)?;
    remote::call(
        reqwest::Method::POST,
        "apps/unpublish",
        Some(account),
        json!({"slug":input["slug"]}),
    )
    .await
    .map_err(invalid)?;
    Ok(Json(json!({"ok":true})))
}

async fn deploy(root: &Path, slug: &str) -> Result<String> {
    let account = remote::account()?;
    let output = output(root)?;
    let files = files(&output)?;
    let manifest: Map<String, Value> = files
        .iter()
        .map(|(path, file)| (path.clone(), json!({"hash":file.hash,"size":file.size})))
        .collect();
    let begun = remote::call(
        reqwest::Method::POST,
        "apps/publish/begin",
        Some(account.clone()),
        json!({"slug":slug,"manifest":manifest}),
    )
    .await?;
    let upload = begun["upload"]
        .as_str()
        .context("the Cryzo cloud gave no upload address")?;
    let session = begun["jwt"].as_str().unwrap_or_default();
    let by_hash: HashMap<&str, &File> = files
        .values()
        .map(|file| (file.hash.as_str(), file))
        .collect();
    // With nothing new to upload, the session token already completes the upload; otherwise
    // the response to the last bucket carries the completion token.
    let mut token = session.to_owned();
    let client = reqwest::Client::new();
    for bucket in begun["buckets"].as_array().into_iter().flatten() {
        let mut form = reqwest::multipart::Form::new();
        for hash in bucket
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let file = by_hash
                .get(hash)
                .with_context(|| format!("Cloudflare asked for an unknown file {hash}"))?;
            let part = reqwest::multipart::Part::text(STANDARD.encode(std::fs::read(&file.path)?))
                .file_name(hash.to_owned())
                .mime_str(content_type(&file.path))?;
            form = form.part(hash.to_owned(), part);
        }
        let response = client
            .post(upload)
            .bearer_auth(session)
            .multipart(form)
            .send()
            .await?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        ensure!(
            status.is_success(),
            "uploading the app failed: {}",
            body["errors"][0]["message"]
                .as_str()
                .unwrap_or("Cloudflare refused the files")
        );
        if let Some(done) = body["result"]["jwt"].as_str() {
            token = done.to_owned();
        }
    }
    let server = worker(root, &output)?;
    let finished = remote::call(
        reqwest::Method::POST,
        "apps/publish/finish",
        Some(account),
        json!({"slug":slug,"jwt":token,"modules":server.modules,"assets":server.assets}),
    )
    .await?;
    finished["url"]
        .as_str()
        .map(str::to_owned)
        .context("the Cryzo cloud did not return the app's address")
}

/// The folder to publish: the build output, or the project itself when it is a plain site.
fn output(root: &Path) -> Result<PathBuf> {
    if let Some(built) = OUTPUTS
        .iter()
        .map(|folder| root.join(folder))
        .find(|folder| folder.join("index.html").is_file())
    {
        return Ok(built);
    }
    if root.join("index.html").is_file() && !root.join("package.json").is_file() {
        return Ok(root.to_path_buf());
    }
    bail!("No built app to publish. Run the build first so dist, build or out has an index.html.")
}

#[derive(Default)]
struct Worker {
    /// Module name and code, the main module first. Empty for a static site.
    modules: Vec<Value>,
    /// How static files and the Worker share requests, from the app's own Wrangler settings.
    assets: Map<String, Value>,
}

/// The app's server code. The Cloudflare Vite plugin builds it into `dist/<name>/` next to the
/// site in `dist/client`, with a `wrangler.json` naming the main module; the older convention is
/// a single `_worker.js` among the static files.
fn worker(root: &Path, output: &Path) -> Result<Worker> {
    let built = std::fs::read_dir(root.join("dist"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .find(|folder| folder != output && folder.join("wrangler.json").is_file());
    let Some(folder) = built else {
        let modules = std::fs::read_to_string(output.join(MODULE))
            .map(|code| vec![json!({"name":"index.js","content":code})])
            .unwrap_or_default();
        return Ok(Worker {
            modules,
            ..Worker::default()
        });
    };
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join("wrangler.json"))?)
            .context("the Worker build has an unreadable wrangler.json")?;
    let main = settings["main"].as_str().unwrap_or("index.js");
    let mut modules = Vec::new();
    let mut pending = vec![folder.clone()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let name = path
                .strip_prefix(&folder)?
                .to_string_lossy()
                .replace('\\', "/");
            if name.ends_with(".js") || name.ends_with(".mjs") {
                modules.push(json!({"name":name,"content":std::fs::read_to_string(&path)?}));
            }
        }
    }
    ensure!(
        modules.iter().any(|module| module["name"] == main),
        "the Worker build is missing its main module {main}"
    );
    modules.sort_by_key(|module| module["name"] != main);
    let assets = ["not_found_handling", "run_worker_first", "html_handling"]
        .into_iter()
        .filter_map(|key| Some((key.to_owned(), settings["assets"].get(key)?.clone())))
        .collect();
    Ok(Worker { modules, assets })
}

struct File {
    path: PathBuf,
    hash: String,
    size: u64,
}

/// The files to upload, keyed by their address in the app, with the hash Cloudflare expects:
/// the first 32 hex characters of SHA-256 over the base64 contents followed by the extension.
fn files(output: &Path) -> Result<HashMap<String, File>> {
    let mut files = HashMap::new();
    let mut pending = vec![output.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if kind.is_symlink() || SKIPPED.contains(&name.as_str()) {
                continue;
            }
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(output)?
                .to_string_lossy()
                .replace('\\', "/");
            // The Worker is uploaded as code, and `.assetsignore` only tells Wrangler what to skip.
            if relative == MODULE || name == ".assetsignore" {
                continue;
            }
            let bytes = std::fs::read(entry.path())?;
            let extension = entry
                .path()
                .extension()
                .map(|extension| extension.to_string_lossy().into_owned())
                .unwrap_or_default();
            let digest = Sha256::digest(format!("{}{extension}", STANDARD.encode(&bytes)));
            files.insert(
                format!("/{relative}"),
                File {
                    path: entry.path(),
                    hash: format!("{digest:x}")[..32].to_owned(),
                    size: bytes.len() as u64,
                },
            );
        }
    }
    ensure!(!files.is_empty(), "The build folder is empty.");
    Ok(files)
}

/// The type Cloudflare serves each file with.
fn content_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "json" | "webmanifest" => "application/json",
        "txt" => "text/plain",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Files that must never be published, whatever they contain.
fn forbidden(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    if lower == ".env" || (lower.starts_with(".env.") && !lower.ends_with(".example")) {
        return Some("environment file: put these values in Cryzo's app secrets instead");
    }
    if lower.ends_with(".map") {
        return Some("source map: it exposes the app's source code");
    }
    if [
        "id_rsa",
        "id_ed25519",
        ".npmrc",
        ".pypirc",
        "credentials.json",
    ]
    .contains(&lower.as_str())
        || [".pem", ".key", ".p12", ".pfx"]
            .iter()
            .any(|extension| lower.ends_with(extension))
    {
        return Some("credential file");
    }
    None
}

/// Text that looks like a real secret rather than a placeholder.
fn secret(text: &str) -> Option<&'static str> {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY-----") {
        return Some("contains a private key");
    }
    let tokens = text.split(|char: char| !(char.is_ascii_alphanumeric() || "-_".contains(char)));
    for token in tokens {
        let starts = |prefix: &str, min: usize| token.starts_with(prefix) && token.len() >= min;
        if starts("AKIA", 20)
            && token[4..]
                .chars()
                .all(|char| char.is_ascii_uppercase() || char.is_ascii_digit())
        {
            return Some("contains an AWS access key");
        }
        if starts("sk_live_", 24) || starts("rk_live_", 24) {
            return Some("contains a live Stripe key");
        }
        if starts("sk-", 40) || starts("sk-ant-", 40) || starts("sk-proj-", 40) {
            return Some("contains an AI provider API key");
        }
        if starts("ghp_", 36) || starts("github_pat_", 40) || starts("xoxb-", 30) {
            return Some("contains an access token");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_files_keys_and_source_maps_block_publishing() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let target = root.path().join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, text).unwrap();
        };
        write("index.html", "<h1>Hello</h1>");
        write(".env", "DATABASE_URL=x");
        write(".env.example", "DATABASE_URL=");
        write("dist/app.js.map", "{}");
        write("keys/server.pem", "x");
        write(
            "src/config.ts",
            "const key = \"sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789\"",
        );
        write("src/stripe.ts", "const key = process.env.STRIPE_SECRET_KEY");
        write("node_modules/pkg/.env", "ignored");
        let problems = check(root.path()).unwrap();
        let paths: Vec<_> = problems
            .iter()
            .map(|problem| problem.path.as_str())
            .collect();
        assert_eq!(
            paths,
            [
                ".env",
                "dist/app.js.map",
                "keys/server.pem",
                "src/config.ts"
            ]
        );
    }

    #[test]
    fn the_build_folder_is_published_with_cloudflare_hashes() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let target = root.path().join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, text).unwrap();
        };
        write("index.html", "<script src=/src/main.ts></script>");
        write("package.json", "{}");
        assert!(
            output(root.path()).is_err(),
            "an unbuilt app is not published"
        );
        write("dist/index.html", "<h1>Hi</h1>");
        write("dist/assets/app.js", "console.log(1)");
        write("dist/_worker.js", "export default {}");
        let built = output(root.path()).unwrap();
        assert_eq!(built, root.path().join("dist"));
        let files = files(&built).unwrap();
        let mut paths: Vec<_> = files.keys().map(String::as_str).collect();
        paths.sort();
        assert_eq!(paths, ["/assets/app.js", "/index.html"]);
        let index = &files["/index.html"];
        let expected = format!(
            "{:x}",
            Sha256::digest(format!("{}html", STANDARD.encode("<h1>Hi</h1>")))
        );
        assert_eq!(index.hash, expected[..32]);
        assert_eq!(index.size, 11);
        assert_eq!(
            content_type(&files["/assets/app.js"].path),
            "application/javascript"
        );
    }

    #[test]
    fn a_vite_worker_build_is_published_as_modules_with_its_routing() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let target = root.path().join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, text).unwrap();
        };
        write("package.json", "{}");
        write("dist/client/index.html", "<h1>Hi</h1>");
        write("dist/client/.assetsignore", "wrangler.json");
        write(
            "dist/app/wrangler.json",
            r#"{"main":"index.js","assets":{"not_found_handling":"single-page-application","run_worker_first":["/api/*"],"directory":"../client"}}"#,
        );
        write("dist/app/index.js", "import('./assets/a.js')");
        write("dist/app/index.js.map", "{}");
        write("dist/app/assets/a.js", "export {}");
        let output = output(root.path()).unwrap();
        assert_eq!(output, root.path().join("dist/client"));
        assert_eq!(
            check(root.path()).unwrap(),
            [],
            "Worker source maps are never served"
        );
        assert_eq!(
            files(&output).unwrap().keys().collect::<Vec<_>>(),
            ["/index.html"]
        );
        let server = worker(root.path(), &output).unwrap();
        let names: Vec<_> = server
            .modules
            .iter()
            .map(|module| module["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["index.js", "assets/a.js"]);
        assert_eq!(
            Value::Object(server.assets),
            json!({"not_found_handling":"single-page-application","run_worker_first":["/api/*"]})
        );
    }

    #[test]
    fn a_plain_site_publishes_its_own_folder() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("index.html"), "<h1>Hi</h1>").unwrap();
        assert_eq!(output(root.path()).unwrap(), root.path());
    }

    #[test]
    fn placeholders_and_ordinary_words_pass() {
        assert_eq!(secret("sk-your-key-here and AKIA"), None);
        assert_eq!(secret("const skip = true"), None);
        assert!(secret("ghp_0123456789abcdefghijklmnopqrstuvwxyz").is_some());
        assert!(secret("-----BEGIN RSA PRIVATE KEY-----\nabc").is_some());
    }
}
