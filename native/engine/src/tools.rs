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
}

impl Operation {
    pub fn needs_approval(&self) -> bool {
        matches!(
            self,
            Self::Write { .. } | Self::Shell { .. } | Self::MarketPost { schedule: true, .. }
        ) || matches!(self, Self::Desktop { command, .. } if cryzo_desktop_control::acts(command))
    }

    pub fn is_desktop(&self) -> bool {
        matches!(self, Self::Desktop { .. })
    }

    pub fn is_market(&self) -> bool {
        matches!(
            self,
            Self::MarketChannels {} | Self::MarketPosts { .. } | Self::MarketPost { .. }
        )
    }
}

pub struct Workspace(PathBuf);

impl Workspace {
    pub fn open(path: &Path) -> Result<Self> {
        let path = path.canonicalize().context("open workspace")?;
        ensure!(path.is_dir(), "workspace must be a directory");
        Ok(Self(path))
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
            Operation::Shell { program, args } => self.process(program, args).await,
            Operation::MarketChannels {}
            | Operation::MarketPosts { .. }
            | Operation::MarketPost { .. } => {
                anyhow::bail!("marketing tools run in the session runner")
            }
            Operation::Desktop { .. } => anyhow::bail!("desktop tools run in the session runner"),
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

    async fn process(&self, program: &str, args: &[String]) -> Result<Value> {
        ensure!(!program.is_empty(), "program is required");
        let mut command = Command::new(program);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        command
            .args(args)
            .current_dir(&self.0)
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

async fn capture(reader: impl tokio::io::AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    reader.take(2_000_001).read_to_end(&mut output).await?;
    ensure!(output.len() <= 2_000_000, "command output exceeded 2 MB");
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::Operation;
    use serde_json::json;

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
