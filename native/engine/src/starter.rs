//! New apps start from the Cryzo app starter (`templates/app-starter`): a React site, a Worker
//! API, a D1 database and Better Auth sign-in, already set up so publishing gives the user a
//! working app with accounts. The files are built into the engine, so a new app needs no download.
use anyhow::{Context, Result};
use axum::{Json, Router, http::StatusCode, routing::post};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

macro_rules! starter {
    ($($path:literal),* $(,)?) => {
        [$(($path, include_str!(concat!("../templates/app-starter/", $path)))),*]
    };
}

const FILES: [(&str, &str); 19] = starter![
    ".gitignore",
    "AGENTS.md",
    "index.html",
    "package.json",
    "tsconfig.client.json",
    "tsconfig.worker.json",
    "vite.config.ts",
    "wrangler.jsonc",
    "migrations/0001_auth.sql",
    "migrations/0002_notes.sql",
    "src/client/App.tsx",
    "src/client/auth.ts",
    "src/client/index.css",
    "src/client/main.tsx",
    "src/worker/auth.ts",
    "src/worker/db.ts",
    "src/worker/env.d.ts",
    "src/worker/index.ts",
    "src/worker/migrate.ts",
];

pub fn router() -> Router {
    Router::new().route("/api/cryzo/apps/new", post(new_app))
}

/// Creates the app in the user's Cryzo Apps folder and returns where, for the app to open.
async fn new_app(Json(input): Json<Value>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let name = input["name"].as_str().unwrap_or_default().trim();
    let created = apps_folder().and_then(|parent| create(&parent, name));
    match created {
        Ok(directory) => Ok(Json(json!({"directory":directory}))),
        Err(error) => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"_tag":"InvalidRequestError","message":format!("{error:#}")})),
        )),
    }
}

/// `Documents/Cryzo Apps`, or `Cryzo Apps` in the home folder when there is no Documents folder.
fn apps_folder() -> Result<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .context("no home folder")?;
    let documents = home.join("Documents");
    let base = if documents.is_dir() { documents } else { home };
    Ok(base.join("Cryzo Apps"))
}

/// Writes the starter into a new folder named after the app, never into an existing one.
pub fn create(parent: &Path, name: &str) -> Result<PathBuf> {
    let name = if name.is_empty() { "My app" } else { name };
    let slug = slug(name);
    std::fs::create_dir_all(parent)?;
    let directory = (1..1000)
        .map(|n| {
            parent.join(if n == 1 {
                slug.clone()
            } else {
                format!("{slug}-{n}")
            })
        })
        .find(|candidate| !candidate.exists())
        .context("too many apps with this name")?;
    for (path, text) in FILES {
        let target = directory.join(path);
        std::fs::create_dir_all(target.parent().context("file without a folder")?)?;
        let text = match path {
            "package.json" => text.replace(r#""name": "app""#, &format!(r#""name": "{slug}""#)),
            "wrangler.jsonc" => text
                .replace(r#""name": "app""#, &format!(r#""name": "{slug}""#))
                .replace(
                    r#""database_name": "app""#,
                    &format!(r#""database_name": "{slug}""#),
                ),
            "index.html" => text.replace(
                "<title>App</title>",
                &format!("<title>{}</title>", html(name)),
            ),
            // A JSON string is a valid JSX expression, whatever the name contains.
            "src/client/App.tsx" => text.replace(">My app<", &format!(">{{{}}}<", json!(name))),
            _ => text.to_owned(),
        };
        std::fs::write(target, text)?;
    }
    Ok(directory)
}

fn slug(name: &str) -> String {
    let mut slug = String::new();
    for char in name.to_lowercase().chars() {
        if char.is_ascii_alphanumeric() {
            slug.push(char);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(40).collect();
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "my-app".to_owned()
    } else {
        slug.to_owned()
    }
}

fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_app_is_the_starter_named_after_it() {
        let parent = tempfile::tempdir().unwrap();
        let first = create(parent.path(), "Habit <Tracker>").unwrap();
        assert_eq!(first, parent.path().join("habit-tracker"));
        let read = |path: &str| std::fs::read_to_string(first.join(path)).unwrap();
        assert!(read("package.json").contains(r#""name": "habit-tracker""#));
        assert!(read("wrangler.jsonc").contains(r#""database_name": "habit-tracker""#));
        assert!(read("index.html").contains("<title>Habit &lt;Tracker&gt;</title>"));
        assert!(read("src/client/App.tsx").contains(r#">{"Habit <Tracker>"}<"#));
        assert!(read("migrations/0001_auth.sql").contains(r#"create table "user""#));
        assert!(first.join(".gitignore").is_file() && first.join("AGENTS.md").is_file());

        let second = create(parent.path(), "habit tracker").unwrap();
        assert_eq!(second, parent.path().join("habit-tracker-2"));
        assert_eq!(slug("  !!  "), "my-app");
    }

    #[test]
    fn the_starter_passes_the_publish_checks() {
        let parent = tempfile::tempdir().unwrap();
        let app = create(parent.path(), "Checked").unwrap();
        assert_eq!(crate::publish::check(&app).unwrap(), []);
    }
}
