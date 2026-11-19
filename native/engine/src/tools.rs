use crate::store::Store;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Read {
        path: String,
    },
    List {
        path: String,
    },
    Write {
        path: String,
        content: String,
    },
    Shell {
        program: String,
        args: Vec<String>,
    },
    Diff,
    /// Marketing tools run against Cryzo cloud in the session runner, not in the workspace.
    MarketChannels {},
    MarketPosts {
        days: u32,
    },
    MarketPost {
        content: String,
        channels: Vec<String>,
        when: String,
        schedule: bool,
    },
    /// Desktop control (text-first UI Automation) runs on the desktop-control thread in the
    /// session runner, not in the workspace. `command` is a desktop-control tool name.
    Desktop {
        command: String,
        args: Value,
    },
    /// The project's dev server, shown in the browser pane. It runs in the session runner.
    Preview {
        program: String,
        args: Vec<String>,
    },
    PreviewLogs {
        lines: Option<usize>,
    },
    PreviewStop {},
    /// The page tools work on the project's running preview in the browser pane.
    PageRead {},
    PageAct {
        element: u32,
        action: String,
        text: Option<String>,
    },
    PageErrors {},
    /// A picture of the preview, for models that see images.
    PageScreenshot {},
}

impl Operation {
    pub fn needs_approval(&self) -> bool {
        matches!(
            self,
            Self::Write { .. }
                | Self::Shell { .. }
                | Self::Preview { .. }
                | Self::MarketPost { schedule: true, .. }
        ) || matches!(self, Self::Desktop { command, .. } if cryzo_desktop_control::acts(command))
    }

    pub fn is_desktop(&self) -> bool {
        matches!(self, Self::Desktop { .. })
    }

    pub fn is_preview(&self) -> bool {
        matches!(
            self,
            Self::Preview { .. }
                | Self::PreviewLogs { .. }
                | Self::PreviewStop {}
                | Self::PageRead {}
                | Self::PageAct { .. }
                | Self::PageErrors {}
                | Self::PageScreenshot {}
        )
    }

    pub fn is_market(&self) -> bool {
        matches!(
            self,
            Self::MarketChannels {} | Self::MarketPosts { .. } | Self::MarketPost { .. }
        )
    }
}

/// A project folder. Files are always read and written here; a folder linked to Cryzo Cloud
/// runs its commands in a cloud workspace (see `cloud.rs`).
pub struct Workspace(PathBuf, Option<String>);

impl Workspace {
    pub fn open(path: &Path) -> Result<Self> {
        let path = path.canonicalize().context("open workspace")?;
        ensure!(path.is_dir(), "workspace must be a directory");
        let cloud = crate::cloud::project(&path);
        Ok(Self(path, cloud))
    }

    /// Resolve existing ancestors too: lexical checks alone do not contain symlinks/junctions.
    pub fn resolve(&self, relative: &str) -> Result<PathBuf> {
        let relative = Path::new(relative);
        ensure!(
            !relative.is_absolute(),
            "absolute paths are not workspace-relative"
        );
        ensure!(
            relative
                .components()
                .all(|part| matches!(part, Component::Normal(_) | Component::CurDir)),
            "path escapes workspace"
        );
        let mut path = self.0.clone();
        for part in relative.components() {
            if part == Component::CurDir {
                continue;
            }
            path.push(part);
            // symlink_metadata detects dangling links too; never write through them.
            if std::fs::symlink_metadata(&path).is_ok() {
                path = path.canonicalize().context("resolve workspace path")?;
                ensure!(
                    path.starts_with(&self.0),
                    "path escapes workspace through a link"
                );
            }
        }
        Ok(path)
    }

    pub async fn execute(
        &self,
        store: &Store,
        session: &str,
        operation: &Operation,
        approval: Option<&str>,
    ) -> Result<Value> {
        if operation.needs_approval() {
            store.consume_approval(
                session,
                approval.context("operation needs approval")?,
                &serde_json::to_value(operation)?,
            )?;
        }
        match operation {
            Operation::Read { path } => {
                let path = self.resolve(path)?;
                ensure!(
                    std::fs::metadata(&path)?.len() <= 2_000_000,
                    "file is too large for a tool read"
                );
                Ok(json!({"content":tokio::fs::read_to_string(path).await?}))
            }
            Operation::List { path } => {
                let mut entries = tokio::fs::read_dir(self.resolve(path)?).await?;
                let mut result = Vec::new();
                while let Some(entry) = entries.next_entry().await? {
                    ensure!(result.len() < 5000, "directory exceeds listing limit");
                    result.push(json!({"name":entry.file_name().to_string_lossy(),"directory":entry.file_type().await?.is_dir()}));
                }
                result.sort_by_key(|entry| entry["name"].as_str().unwrap_or_default().to_owned());
                Ok(json!(result))
            }
            Operation::Write { path, content } => {
                ensure!(content.len() <= 2_000_000, "write exceeds size limit");
                let path = self.resolve(path)?;
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(path, content).await?;
                Ok(json!({"written":true}))
            }
            Operation::Shell { program, args } => match &self.1 {
                Some(project) => crate::cloud::run(&self.0, project, program, args).await,
                None => self.process(program, args).await,
            },
            Operation::MarketChannels {}
            | Operation::MarketPosts { .. }
            | Operation::MarketPost { .. } => {
                anyhow::bail!("marketing tools run in the session runner")
            }
            Operation::Desktop { .. } => anyhow::bail!("desktop tools run in the session runner"),
            Operation::Preview { .. }
            | Operation::PreviewLogs { .. }
            | Operation::PreviewStop {}
            | Operation::PageRead {}
            | Operation::PageAct { .. }
            | Operation::PageErrors {}
            | Operation::PageScreenshot {} => {
                anyhow::bail!("preview tools run in the session runner")
            }
            Operation::Diff => {
                self.process(
                    "git",
                    &[
                        "--no-pager".into(),
                        "diff".into(),
                        "--no-ext-diff".into(),
                        "--no-textconv".into(),
                    ],
                )
                .await
            }
        }
    }

    pub fn root(&self) -> &Path {
        &self.0
    }

    async fn process(&self, program: &str, args: &[String]) -> Result<Value> {
        let mut command = command(program, args, &self.0)?;
        command
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().context("command has no stdout")?;
        let stderr = child.stderr.take().context("command has no stderr")?;
        let output = tokio::time::timeout(std::time::Duration::from_secs(120), async {
            let (stdout, stderr) = tokio::try_join!(capture(stdout), capture(stderr))?;
            let status = child.wait().await?;
            Ok::<_, anyhow::Error>((status, stdout, stderr))
        })
        .await
        .context("command exceeded 120 seconds")??;
        Ok(
            json!({"exit_code":output.0.code(),"stdout":String::from_utf8_lossy(&output.1),"stderr":String::from_utf8_lossy(&output.2)}),
        )
    }
}

/// A command for `program` with `args` in the project folder, without a console window.
/// On Windows a bare name is looked up on PATH with PATHEXT as a shell would (so `npm` finds
/// `npm.cmd`), never in the project folder itself, so a file there can't stand in for a real
/// program. Rust runs `.cmd`/`.bat` files through `cmd.exe` with safe escaping and refuses
/// arguments it can't escape.
pub(crate) fn command(program: &str, args: &[String], root: &Path) -> Result<Command> {
    ensure!(!program.is_empty(), "program is required");
    #[cfg(windows)]
    let program = resolve(
        program,
        root,
        &std::env::var_os("PATH").unwrap_or_default(),
        &std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into()),
    )
    .with_context(|| format!("program not found: {program}"))?;
    let mut command = Command::new(program);
    command.args(args).current_dir(root);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    Ok(command)
}

/// The file a Windows shell would run for `program`: a path (relative to the project) with a
/// PATHEXT extension added when it has none, or the first match in the PATH folders.
#[cfg(any(windows, test))]
fn resolve(program: &str, root: &Path, path: &std::ffi::OsStr, pathext: &str) -> Option<PathBuf> {
    let extensions: Vec<&str> = pathext
        .split(';')
        .filter(|extension| !extension.is_empty())
        .collect();
    let candidates = |base: PathBuf| -> Vec<PathBuf> {
        if base.extension().is_some() {
            return vec![base];
        }
        extensions
            .iter()
            .map(|extension| base.with_extension(extension.trim_start_matches('.')))
            .collect()
    };
    if program.contains(['/', '\\']) {
        return candidates(root.join(program))
            .into_iter()
            .find(|path| path.is_file());
    }
    std::env::split_paths(path)
        .flat_map(|folder| candidates(folder.join(program)))
        .find(|path| path.is_file())
}

async fn capture(reader: impl tokio::io::AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    reader.take(2_000_001).read_to_end(&mut output).await?;
    ensure!(output.len() <= 2_000_000, "command output exceeded 2 MB");
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{Operation, resolve};
    use serde_json::json;

    #[test]
    fn bare_names_find_scripts_on_path_like_a_shell() {
        let folder = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("npm.cmd"), "@echo off").unwrap();
        std::fs::write(folder.path().join("node.exe"), "").unwrap();
        // A same-named file in the project never stands in for the real program.
        std::fs::write(project.path().join("node.cmd"), "").unwrap();
        std::fs::create_dir_all(project.path().join("bin")).unwrap();
        std::fs::write(project.path().join("bin/tool.cmd"), "").unwrap();
        let path = std::env::join_paths([folder.path()]).unwrap();
        let ext = ".COM;.EXE;.BAT;.CMD";
        let found = |program: &str| {
            resolve(program, project.path(), &path, ext)
                .map(|found| found.to_string_lossy().to_lowercase().replace('\\', "/"))
        };
        let expect = |path: std::path::PathBuf| {
            Some(path.to_string_lossy().to_lowercase().replace('\\', "/"))
        };
        assert_eq!(found("npm"), expect(folder.path().join("npm.cmd")));
        assert_eq!(found("node"), expect(folder.path().join("node.exe")));
        assert_eq!(found("npm.cmd"), expect(folder.path().join("npm.cmd")));
        assert_eq!(
            found("bin/tool"),
            expect(project.path().join("bin/tool.cmd"))
        );
        assert_eq!(found("missing"), None);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn batch_files_get_their_arguments_literally() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join("greet.cmd"),
            // %1 keeps the quotes Rust added, so `echo` prints the arguments without running them.
            "@echo off\r\necho %1^|%2\r\n",
        )
        .unwrap();
        let output = super::command(
            "./greet",
            &["big world".into(), "a&calc".into()],
            folder.path(),
        )
        .unwrap()
        .output()
        .await
        .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            r#""big world"|"a&calc""#
        );
    }

    #[test]
    fn desktop_actions_need_approval_and_reads_do_not() {
        let operation = |command: &str| {
            serde_json::from_value::<Operation>(
                json!({"tool":"desktop","command":command,"args":{}}),
            )
            .unwrap()
        };
        for command in ["act", "key", "focus", "launch", "click_at"] {
            assert!(operation(command).needs_approval(), "{command}");
        }
        for command in ["windows", "snapshot", "find", "ocr", "wait_for"] {
            assert!(!operation(command).needs_approval(), "{command}");
        }
    }
}
