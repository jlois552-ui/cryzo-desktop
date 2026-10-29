use crate::{service::Engine, tools::Workspace};
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, RawQuery, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/api/fs/list", get(list))
        .route("/api/fs/find", get(find))
        .route("/api/fs/read/{*path}", get(read))
        .route("/api/experimental/fs/write", post(write))
        .route("/api/vcs", get(vcs))
        .route("/api/vcs/status", get(status))
        .route("/api/vcs/diff", get(diff))
        .route("/api/worktree", get(worktrees))
        .route("/api/session/{id}/diff", get(session_diff))
        .with_state(engine)
}
type Error = (StatusCode, Json<Value>);
type ApiResult = Result<Json<Value>, Error>;
fn error(error: impl ToString) -> Error {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"_tag":"InvalidRequestError","message":error.to_string()})),
    )
}
fn query(raw: &Option<String>) -> HashMap<String, String> {
    url::form_urlencoded::parse(raw.as_deref().unwrap_or_default().as_bytes())
        .into_owned()
        .collect()
}
fn locate(root: &std::path::Path, data: Value) -> Json<Value> {
    Json(
        json!({"location":{"directory":root.to_string_lossy().trim_start_matches(r"\\?\")},"data":data}),
    )
}
fn resolve(root: &std::path::Path, path: &str) -> Result<PathBuf> {
    let root = PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\"));
    let path = PathBuf::from(path.trim_start_matches(r"\\?\"));
    let relative = if path.is_absolute() {
        path.strip_prefix(&root)
            .context("file is outside this project")?
            .to_string_lossy()
            .into_owned()
    } else {
        path.to_string_lossy().into_owned()
    };
    Workspace::open(&root)?.resolve(&relative)
}
async fn list(RawQuery(raw): RawQuery) -> ApiResult {
    let root = crate::desktop_api::directory(raw.as_deref()).map_err(error)?;
    let path = resolve(
        &root,
        query(&raw).get("path").map(String::as_str).unwrap_or("."),
    )
    .map_err(error)?;
    let mut entries = tokio::fs::read_dir(path).await.map_err(error)?;
    let mut data = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(error)? {
        let resolved = entry.path().canonicalize().map_err(error)?;
        if !resolved.starts_with(&root) {
            continue;
        }
        let path = entry
            .path()
            .strip_prefix(&root)
            .map_err(error)?
            .to_string_lossy()
            .replace('\\', "/");
        data.push(json!({"path":path,"type":if entry.file_type().await.map_err(error)?.is_dir(){"directory"}else{"file"}}));
    }
    data.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(locate(&root, json!(data)))
}
async fn find(RawQuery(raw): RawQuery) -> ApiResult {
    let root = crate::desktop_api::directory(raw.as_deref()).map_err(error)?;
    let q = query(&raw);
    let text = q.get("query").cloned().unwrap_or_default().to_lowercase();
    let limit = q
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 1000);
    let kind = q.get("type");
    let mut queue = std::collections::VecDeque::from([root.clone()]);
    let mut data = Vec::new();
    let mut scanned = 0;
    while let Some(dir) = queue.pop_front() {
        let mut entries = tokio::fs::read_dir(dir).await.map_err(error)?;
        while let Some(entry) = entries.next_entry().await.map_err(error)? {
            scanned += 1;
            if scanned > 20000 || data.len() >= limit {
                return Ok(locate(&root, json!(data)));
            }
            let name = entry.file_name();
            if [".git", "node_modules", "target"].contains(&name.to_string_lossy().as_ref()) {
                continue;
            }
            let file_type = entry.file_type().await.map_err(error)?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry
                .path()
                .strip_prefix(&root)
                .map_err(error)?
                .to_string_lossy()
                .replace('\\', "/");
            let file_kind = if file_type.is_dir() {
                "directory"
            } else {
                "file"
            };
            if path.to_lowercase().contains(&text) && kind.is_none_or(|k| k == file_kind) {
                data.push(json!({"path":path,"type":file_kind}));
            }
            if file_type.is_dir() {
                queue.push_back(entry.path());
            }
        }
    }
    Ok(locate(&root, json!(data)))
}
async fn read(
    Path(path): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<impl IntoResponse, Error> {
    let root = crate::desktop_api::directory(raw.as_deref()).map_err(error)?;
    let path = resolve(&root, &path).map_err(error)?;
    let size = tokio::fs::metadata(&path).await.map_err(error)?.len();
    if size > 20_000_000 {
        return Err(error("file exceeds the 20 MB preview limit"));
    }
    let bytes = tokio::fs::read(path).await.map_err(error)?;
    Ok(([("content-type", "application/octet-stream")], bytes))
}
async fn write(
    State(engine): State<Arc<Engine>>,
    RawQuery(raw): RawQuery,
    body: Bytes,
) -> ApiResult {
    let root = crate::desktop_api::directory(raw.as_deref()).map_err(error)?;
    let q = query(&raw);
    let input = q.get("path").ok_or_else(|| error("path missing"))?;
    let path = resolve(&root, input).map_err(error)?;
    if body.len() > 4_000_000 {
        return Err(error("file exceeds edit limit"));
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(error)?;
    }
    tokio::fs::write(&path, body).await.map_err(error)?;
    let mut event = crate::desktop_api::event(
        "filesystem.changed",
        json!({"file":path.strip_prefix(&root).map_err(error)?.to_string_lossy(),"event":"change"}),
    );
    event["location"] = json!({"directory":root.to_string_lossy().trim_start_matches(r"\\?\")});
    engine.emit(event);
    Ok(locate(
        &root,
        json!({"path":path.to_string_lossy().trim_start_matches(r"\\?\")}),
    ))
}
async fn git(root: &std::path::Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-c")
        .arg("core.fsmonitor=false")
        .args(args)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output =
        tokio::time::timeout(std::time::Duration::from_secs(20), command.output()).await??;
    ensure!(output.status.success(), "Git command failed");
    ensure!(output.stdout.len() < 20_000_000, "Git output exceeds limit");
    Ok(output.stdout)
}
async fn vcs(RawQuery(raw): RawQuery) -> ApiResult {
    let root = crate::desktop_api::directory(raw.as_deref()).map_err(error)?;
    let current = git(&root, &["branch", "--show-current"]).await.ok();
    let mut info = json!({"branch":{}});
    if let Some(current) = current {
        info["provider"] = json!("git");
        let current = String::from_utf8_lossy(&current).trim().to_owned();
        if !current.is_empty() {
            info["branch"]["current"] = json!(current);
        }
    }
    Ok(locate(&root, info))
}
async fn worktrees(State(engine): State<Arc<Engine>>, RawQuery(raw): RawQuery) -> ApiResult {
    let q = query(&raw);
    let id = q
        .get("projectID")
        .ok_or_else(|| error("projectID missing"))?;
    let project = engine
        .store
        .desktop_projects()
        .map_err(error)?
        .into_iter()
        .find(|project| project["id"].as_str() == Some(id))
        .ok_or_else(|| error("project not found"))?;
    let root = std::path::Path::new(
        project["canonical"]
            .as_str()
            .ok_or_else(|| error("project directory missing"))?,
    );
    let mut data = Vec::new();
    if let Ok(output) = git(root, &["worktree", "list", "--porcelain"]).await {
        for line in String::from_utf8_lossy(&output).lines() {
            if let Some(directory) = line.strip_prefix("worktree ")
                && std::path::Path::new(directory).canonicalize().ok() != root.canonicalize().ok()
            {
                data.push(json!({"directory":directory,"strategy":"git"}));
            }
        }
    }
    Ok(Json(json!(data)))
}
pub async fn changes(root: &std::path::Path) -> Result<Vec<Value>> {
    let raw = match git(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
    .await
    {
        Ok(raw) => raw,
        Err(error) => {
            if git(root, &["rev-parse", "--git-dir"]).await.is_err() {
                return Ok(vec![]);
            }
            return Err(error);
        }
    };
    let mut fields = raw.split(|byte| *byte == 0);
    let mut data = Vec::new();
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        let code = String::from_utf8_lossy(&field[..2]);
        let file = String::from_utf8_lossy(&field[3..]).into_owned();
        if code.contains('R') || code.contains('C') {
            fields.next();
        }
        let status = if code.contains('D') {
            "deleted"
        } else if code.contains('A') || code == "??" {
            "added"
        } else {
            "modified"
        };
        let patch = if code == "??" {
            let content = tokio::fs::read_to_string(resolve(root, &file)?)
                .await
                .unwrap_or_default();
            format!(
                "--- /dev/null\n+++ b/{file}\n@@ -0,0 +1,{} @@\n{}",
                content.lines().count(),
                content
                    .lines()
                    .map(|line| format!("+{line}\n"))
                    .collect::<String>()
            )
        } else {
            let args = if git(root, &["rev-parse", "--verify", "HEAD"]).await.is_ok() {
                vec![
                    "diff",
                    "HEAD",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--",
                    file.as_str(),
                ]
            } else {
                vec![
                    "diff",
                    "--cached",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--",
                    file.as_str(),
                ]
            };
            String::from_utf8_lossy(&git(root, &args).await?).into_owned()
        };
        let additions = patch
            .lines()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .count();
        let deletions = patch
            .lines()
            .filter(|line| line.starts_with('-') && !line.starts_with("---"))
            .count();
        data.push(json!({"file":file,"status":status,"patch":patch,"additions":additions,"deletions":deletions}));
    }
    Ok(data)
}
async fn status(RawQuery(raw): RawQuery) -> ApiResult {
    let root = crate::desktop_api::directory(raw.as_deref()).map_err(error)?;
    let mut changes = changes(&root).await.map_err(error)?;
    for item in &mut changes {
        item.as_object_mut().unwrap().remove("patch");
    }
    Ok(locate(&root, json!(changes)))
}
async fn diff(RawQuery(raw): RawQuery) -> ApiResult {
    let root = crate::desktop_api::directory(raw.as_deref()).map_err(error)?;
    if query(&raw)
        .get("mode")
        .is_some_and(|mode| mode != "working")
    {
        return Err(error("choose the working-tree diff for this native build"));
    }
    Ok(locate(&root, json!(changes(&root).await.map_err(error)?)))
}
async fn session_diff(State(engine): State<Arc<Engine>>, Path(id): Path<String>) -> ApiResult {
    let state = engine.store.desktop_state(&id).map_err(error)?;
    let root = std::path::Path::new(
        state["info"]["location"]["directory"]
            .as_str()
            .ok_or_else(|| error("no session directory"))?,
    );
    Ok(Json(json!({"data":changes(root).await.map_err(error)?})))
}
