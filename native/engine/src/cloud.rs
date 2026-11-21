//! Cloud projects: the files stay in the project folder on this computer, and commands run in a
//! Cryzo cloud workspace (services/cloud). Before each command, files that changed here go up;
//! after it, files the command created or changed come back, and files it deleted are deleted
//! here. So the file tree, diffs and git keep working on local files, while installs, builds and
//! dev servers run in the cloud and are previewed on their own hostnames.
//!
//! A folder is a cloud project when it holds `.cryzo/cloud.json`. The gateway enforces the
//! user's daily and monthly cloud minutes when it hands out a workspace ticket.
use crate::{remote, service::Engine};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    extract::{RawQuery, State},
    http::StatusCode,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Never synced: dependencies, git and framework caches. Build output (dist, build) does sync
/// back, because publishing uploads it from this computer. Keep in sync with
/// services/cloud/src/route.ts.
const SKIPPED: [&str; 7] = [
    "node_modules",
    ".git",
    ".cryzo",
    ".next",
    ".svelte-kit",
    "target",
    ".turbo",
];
/// Guidance for the agent when a project runs in Cryzo Cloud.
pub const INSTRUCTIONS: &str = "This project runs in Cryzo Cloud. File reads and writes happen in the project folder; \
every command you run executes in a Linux cloud workspace (Node 22, git, npm) after the folder syncs there, \
and files the command creates or changes sync back; dependencies (node_modules) and framework caches (.next) stay in the cloud. \
Start dev servers in the background and bind them to 0.0.0.0, for example: \
nohup npm run dev -- --host 0.0.0.0 --port 5173 > /tmp/dev.log 2>&1 & \
then tell the user to press Preview with that port. Do not run commands that wait forever in the foreground. \
To put the app online, run the build so dist/index.html exists, then call the publish tool, which shows the user a Publish card to click. \
Published apps run on Cloudflare Workers: files in dist are served as they are, and an optional server module \
at dist/_worker.js (one bundled ES module, `export default { fetch(request, env) }`) handles other requests, \
falling back to env.ASSETS.fetch(request). The server gets env.DB (this app's Cloudflare D1 SQLite database), \
env.BETTER_AUTH_SECRET and env.APP_URL; use Better Auth on env.DB for email and Google sign-in. \
Never put secrets in files: .env files, keys and secrets in code block publishing.";

const MAX_FILES: usize = 5000;
const MAX_FILE: u64 = 10 * 1024 * 1024;

type Hashes = BTreeMap<String, String>;

/// The cloud project a folder is linked to, if any.
pub fn project(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(".cryzo").join("cloud.json")).ok()?;
    serde_json::from_str::<Value>(&text).ok()?["project"]
        .as_str()
        .map(str::to_owned)
}

/// A workspace's API: its address and the ticket that opens it.
#[derive(Clone)]
pub struct Remote {
    pub url: String,
    pub token: String,
    pub preview: String,
    expires: Instant,
}

impl Remote {
    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        let response = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()?
            .post(format!("{}/{path}", self.url))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("could not reach the cloud workspace")?;
        let status = response.status();
        let value: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            bail!(
                "{}",
                value["error"]
                    .as_str()
                    .unwrap_or("The cloud workspace refused the request.")
            );
        }
        Ok(value)
    }

    async fn hashes(&self) -> Result<Hashes> {
        Ok(serde_json::from_value(
            self.post("fs/hashes", json!({})).await?,
        )?)
    }

    async fn download(&self, root: &Path, path: &str) -> Result<()> {
        let data = self.post("fs/read", json!({"path":path})).await?;
        let bytes = STANDARD.decode(data["content"].as_str().context("no content")?)?;
        let target = local(root, path)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, bytes)?;
        Ok(())
    }

    async fn upload(&self, root: &Path, path: &str) -> Result<()> {
        let bytes = std::fs::read(local(root, path)?)?;
        self.post(
            "fs/write",
            json!({"path":path,"content":STANDARD.encode(bytes)}),
        )
        .await
        .map(|_| ())
    }
}

/// Runs a command for a cloud project: sync up, run in the workspace, sync back.
pub async fn run(root: &Path, project: &str, program: &str, args: &[String]) -> Result<Value> {
    let remote = workspace(project).await?;
    run_with(&remote, root, &command(program, args)).await
}

pub(crate) async fn run_with(remote: &Remote, root: &Path, command: &str) -> Result<Value> {
    let known = load(root);
    let ours = local_hashes(root)?;
    let cloud = remote.hashes().await?;
    for (path, hash) in &ours {
        if cloud.get(path) != Some(hash) {
            remote.upload(root, path).await?;
        }
    }
    for path in cloud.keys().filter(|path| !ours.contains_key(*path)) {
        if known.contains_key(path) {
            // Deleted here since the last sync.
            remote.post("fs/delete", json!({"path":path})).await?;
        } else {
            // Made in the cloud, for example in its terminal.
            remote.download(root, path).await?;
        }
    }
    let before = remote.hashes().await?;
    let output = remote
        .post("exec", json!({"command":command,"timeout_ms":600_000}))
        .await?;
    let after = remote.hashes().await?;
    let here = local_hashes(root)?;
    for (path, hash) in &after {
        if here.get(path) != Some(hash) {
            remote.download(root, path).await?;
        }
    }
    for path in before.keys().filter(|path| !after.contains_key(*path)) {
        let _ = std::fs::remove_file(local(root, path)?);
    }
    save(root, &after)?;
    Ok(json!({
        "exit_code": output["code"],
        "stdout": output["stdout"],
        "stderr": output["stderr"],
        "ran_in": "cloud",
    }))
}

/// One bash command line from the tool's program and arguments.
fn command(program: &str, args: &[String]) -> String {
    let quote = |part: &str| {
        if !part.is_empty()
            && part
                .chars()
                .all(|char| char.is_ascii_alphanumeric() || "-_./=:@+,".contains(char))
        {
            part.to_owned()
        } else {
            format!("'{}'", part.replace('\'', r"'\''"))
        }
    };
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A project-relative path from the cloud, as a path inside `root`.
fn local(root: &Path, path: &str) -> Result<PathBuf> {
    let relative = Path::new(path);
    ensure!(
        relative
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_))),
        "cloud path escapes the project"
    );
    Ok(root.join(relative))
}

fn local_hashes(root: &Path) -> Result<Hashes> {
    let mut hashes = Hashes::new();
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
            } else if kind.is_file() && entry.metadata()?.len() <= MAX_FILE {
                ensure!(
                    hashes.len() < MAX_FILES,
                    "cloud projects sync at most {MAX_FILES} files"
                );
                let relative = entry
                    .path()
                    .strip_prefix(root)?
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                let digest = Sha256::digest(std::fs::read(entry.path())?);
                hashes.insert(
                    relative,
                    digest.iter().map(|byte| format!("{byte:02x}")).collect(),
                );
            }
        }
    }
    Ok(hashes)
}

fn manifest(root: &Path) -> PathBuf {
    root.join(".cryzo").join("cloud-manifest.json")
}

fn load(root: &Path) -> Hashes {
    std::fs::read_to_string(manifest(root))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(root: &Path, hashes: &Hashes) -> Result<()> {
    std::fs::create_dir_all(root.join(".cryzo"))?;
    std::fs::write(manifest(root), serde_json::to_string(hashes)?)?;
    Ok(())
}

/// Workspace tickets by project, reused until a few minutes before they expire.
static WORKSPACES: Mutex<Option<HashMap<String, Remote>>> = Mutex::new(None);

async fn workspace(project: &str) -> Result<Remote> {
    // The lock is released before any await.
    let cached = WORKSPACES
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .get(project)
        .filter(|remote| remote.expires > Instant::now() + Duration::from_secs(120))
        .cloned();
    if let Some(remote) = cached {
        // Keeps the gateway's idle timer and minute count current.
        let beat = remote::call(
            reqwest::Method::POST,
            "cloud/heartbeat",
            Some(remote::account()?),
            json!({"project":project}),
        )
        .await?;
        if beat["running"] == false {
            WORKSPACES
                .lock()
                .unwrap()
                .as_mut()
                .map(|map| map.remove(project));
            bail!(
                "{}",
                beat["reason"]
                    .as_str()
                    .unwrap_or("The cloud workspace stopped. Run the command again to restart it.")
            );
        }
        return Ok(remote);
    }
    let data = remote::call(
        reqwest::Method::POST,
        "cloud/start",
        Some(remote::account()?),
        json!({"project":project}),
    )
    .await?;
    let minutes = data["minutes"].as_u64().unwrap_or(5).clamp(1, 60);
    let remote = Remote {
        url: data["url"].as_str().context("no workspace URL")?.to_owned(),
        token: data["token"]
            .as_str()
            .context("no workspace ticket")?
            .to_owned(),
        preview: data["preview"].as_str().unwrap_or_default().to_owned(),
        expires: Instant::now() + Duration::from_secs(minutes * 60),
    };
    WORKSPACES
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(project.to_owned(), remote.clone());
    Ok(remote)
}

/// Settings and preview endpoints the app uses for a project folder.
pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/api/cryzo/cloud", get(status))
        .route("/api/cryzo/cloud/link", post(link))
        .route("/api/cryzo/cloud/unlink", post(unlink))
        .route("/api/cryzo/cloud/preview", post(preview))
        .with_state(engine)
}

type ApiResult = std::result::Result<Json<Value>, (StatusCode, Json<Value>)>;

fn invalid(error: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"_tag":"InvalidRequestError","message":format!("{error:#}")})),
    )
}

async fn status(RawQuery(query): RawQuery) -> ApiResult {
    let root = crate::desktop_api::directory(query.as_deref()).map_err(invalid)?;
    Ok(Json(json!({"linked":project(&root).is_some()})))
}

async fn link(State(_engine): State<Arc<Engine>>, RawQuery(query): RawQuery) -> ApiResult {
    let root = crate::desktop_api::directory(query.as_deref()).map_err(invalid)?;
    let project = match project(&root) {
        Some(project) => project,
        None => {
            let project = uuid::Uuid::new_v4().to_string();
            std::fs::create_dir_all(root.join(".cryzo")).map_err(invalid)?;
            std::fs::write(
                root.join(".cryzo").join("cloud.json"),
                json!({"project":project}).to_string(),
            )
            .map_err(invalid)?;
            project
        }
    };
    let remote = workspace(&project).await.map_err(invalid)?;
    Ok(Json(json!({"linked":true,"preview":remote.preview})))
}

async fn unlink(RawQuery(query): RawQuery) -> ApiResult {
    let root = crate::desktop_api::directory(query.as_deref()).map_err(invalid)?;
    if let Some(project) = project(&root) {
        WORKSPACES
            .lock()
            .unwrap()
            .as_mut()
            .map(|map| map.remove(&project));
        // Stopping is best effort; an idle workspace stops on its own after 10 minutes.
        let _ = remote::call(
            reqwest::Method::POST,
            "cloud/stop",
            Some(remote::account().map_err(invalid)?),
            json!({"project":project}),
        )
        .await;
        std::fs::remove_file(root.join(".cryzo").join("cloud.json")).map_err(invalid)?;
    }
    Ok(Json(json!({"linked":false})))
}

/// A preview address for a dev server port, carrying the ticket once for the preview's cookie.
async fn preview(RawQuery(query): RawQuery, Json(input): Json<Value>) -> ApiResult {
    let root = crate::desktop_api::directory(query.as_deref()).map_err(invalid)?;
    let project = project(&root).ok_or_else(|| invalid("this project is not in the cloud"))?;
    let port = input["port"]
        .as_u64()
        .filter(|port| (1024..=65535).contains(port))
        .ok_or_else(|| invalid("choose a port from 1024 to 65535"))?;
    let remote = workspace(&project).await.map_err(invalid)?;
    let mut url =
        url::Url::parse(&remote.preview.replace("{port}", &port.to_string())).map_err(invalid)?;
    url.query_pairs_mut().append_pair("t", &remote.token);
    Ok(Json(json!({"url":url.as_str()})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path as UrlPath;

    #[test]
    fn commands_are_quoted_for_bash() {
        assert_eq!(command("npm", &["run".into(), "dev".into()]), "npm run dev");
        assert_eq!(
            command("echo", &["it's $HOME".into()]),
            r"echo 'it'\''s $HOME'"
        );
    }

    #[test]
    fn cloud_paths_cannot_leave_the_project() {
        let root = Path::new("project");
        assert!(local(root, "src/app.ts").is_ok());
        for path in ["../x", "/etc/passwd", "src/../../x"] {
            assert!(local(root, path).is_err(), "{path}");
        }
    }

    /// A workspace stand-in over a folder: `exec` makes `made-in-cloud.txt` and removes
    /// `remove-me.txt`, as a build or a generator would.
    async fn fixture(cloud: PathBuf) -> String {
        async fn handle(
            State(cloud): State<PathBuf>,
            UrlPath(action): UrlPath<String>,
            Json(input): Json<Value>,
        ) -> Json<Value> {
            let path = |input: &Value| cloud.join(input["path"].as_str().unwrap_or_default());
            Json(match action.as_str() {
                "hashes" => json!(local_hashes(&cloud).unwrap()),
                "read" => json!({"content":STANDARD.encode(std::fs::read(path(&input)).unwrap())}),
                "write" => {
                    let target = path(&input);
                    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                    std::fs::write(
                        target,
                        STANDARD.decode(input["content"].as_str().unwrap()).unwrap(),
                    )
                    .unwrap();
                    json!({})
                }
                "delete" => {
                    std::fs::remove_file(path(&input)).unwrap();
                    json!({})
                }
                _ => json!({}),
            })
        }
        async fn exec(State(cloud): State<PathBuf>) -> Json<Value> {
            std::fs::write(cloud.join("made-in-cloud.txt"), "built").unwrap();
            let _ = std::fs::remove_file(cloud.join("remove-me.txt"));
            Json(json!({"code":0,"stdout":"ok","stderr":""}))
        }
        let app = Router::new()
            .route("/exec", post(exec))
            .route("/fs/{action}", post(handle))
            .with_state(cloud);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn a_command_syncs_up_runs_in_the_cloud_and_syncs_back() {
        let here = tempfile::tempdir().unwrap();
        let cloud = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(here.path().join("src")).unwrap();
        std::fs::write(here.path().join("src/app.ts"), "local").unwrap();
        std::fs::write(here.path().join("remove-me.txt"), "old").unwrap();
        std::fs::create_dir_all(here.path().join("node_modules/x")).unwrap();
        std::fs::write(here.path().join("node_modules/x/index.js"), "skip").unwrap();
        let remote = Remote {
            url: fixture(cloud.path().to_path_buf()).await,
            token: "t".into(),
            preview: String::new(),
            expires: Instant::now() + Duration::from_secs(600),
        };

        let output = run_with(&remote, here.path(), "npm run build")
            .await
            .unwrap();
        assert_eq!(output["exit_code"], 0);
        assert_eq!(output["ran_in"], "cloud");
        // Up: the local source reached the cloud; dependencies did not.
        assert_eq!(
            std::fs::read_to_string(cloud.path().join("src/app.ts")).unwrap(),
            "local"
        );
        assert!(!cloud.path().join("node_modules").exists());
        // Back: the command's new file came down and its deletion applied here.
        assert_eq!(
            std::fs::read_to_string(here.path().join("made-in-cloud.txt")).unwrap(),
            "built"
        );
        assert!(!here.path().join("remove-me.txt").exists());

        // A file deleted here since the last sync is deleted in the cloud, not brought back.
        std::fs::remove_file(here.path().join("made-in-cloud.txt")).unwrap();
        std::fs::write(
            cloud.path().join("terminal-note.txt"),
            "from the cloud terminal",
        )
        .unwrap();
        run_with(&remote, here.path(), "true").await.unwrap();
        assert_eq!(
            std::fs::read_to_string(here.path().join("terminal-note.txt")).unwrap(),
            "from the cloud terminal"
        );
        // The fixture's exec recreates made-in-cloud.txt, which then syncs back.
        assert!(here.path().join("made-in-cloud.txt").exists());
    }
}
