//! Native PTYs used by the unchanged desktop terminal. The Tauri host carries
//! the socket contract over IPC so no unauthenticated loopback socket is exposed.
use anyhow::{Context, Result, ensure};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Output {
    pub start: u64,
    pub text: String,
}
struct Buffer {
    chunks: VecDeque<Output>,
    cursor: u64,
    bytes: usize,
}
pub struct Pty {
    pub directory: PathBuf,
    info: Mutex<Value>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    buffer: Mutex<Buffer>,
    pub output: broadcast::Sender<Output>,
    reader_done: std::sync::atomic::AtomicBool,
}
struct Ticket {
    pty: String,
    directory: PathBuf,
    expires: Instant,
}
pub struct PtyManager {
    sessions: Mutex<HashMap<String, Arc<Pty>>>,
    tickets: Mutex<HashMap<String, Ticket>>,
    events: broadcast::Sender<Value>,
}

impl PtyManager {
    pub fn new(events: broadcast::Sender<Value>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            tickets: Mutex::new(HashMap::new()),
            events,
        }
    }
    pub fn create(&self, directory: &Path, input: &Value) -> Result<Value> {
        let directory = directory.canonicalize()?;
        ensure!(
            self.sessions.lock().unwrap().len() < 32,
            "too many terminals"
        );
        let cwd = input["cwd"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.clone())
            .canonicalize()?;
        ensure!(
            cwd.starts_with(&directory),
            "terminal cwd is outside this project"
        );
        let command = input["command"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "powershell.exe".into()
                } else {
                    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
                }
            });
        let args: Vec<String> = match input.get("args") {
            Some(args) => serde_json::from_value(args.clone())?,
            None if cfg!(windows) && command == "powershell.exe" => {
                vec!["-NoLogo".into(), "-NoProfile".into()]
            }
            None => vec![],
        };
        let pair = native_pty_system().openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut builder = CommandBuilder::new(&command);
        builder.args(&args);
        builder.cwd(&cwd);
        builder.env("TERM", "xterm-256color");
        if let Some(env) = input["env"].as_object() {
            for (key, value) in env {
                builder.env(
                    key,
                    value
                        .as_str()
                        .context("terminal env must contain strings")?,
                );
            }
        }
        let mut child = pair.slave.spawn_command(builder)?;
        drop(pair.slave);
        let id = format!("pty_{}", Uuid::new_v4());
        let info = json!({"id":id,"title":input["title"].as_str().unwrap_or("Terminal"),"command":command,"args":args,
            "cwd":cwd.to_string_lossy().trim_start_matches(r"\\?\"),"status":"running","pid":child.process_id().unwrap_or(0)});
        let mut reader = pair.master.try_clone_reader()?;
        let pty = Arc::new(Pty {
            directory: directory.clone(),
            info: Mutex::new(info.clone()),
            writer: Mutex::new(pair.master.take_writer()?),
            killer: Mutex::new(child.clone_killer()),
            master: Mutex::new(pair.master),
            buffer: Mutex::new(Buffer {
                chunks: VecDeque::new(),
                cursor: 0,
                bytes: 0,
            }),
            output: broadcast::channel(512).0,
            reader_done: std::sync::atomic::AtomicBool::new(false),
        });
        self.sessions
            .lock()
            .unwrap()
            .insert(id.clone(), pty.clone());
        let mut created = crate::desktop_api::event("pty.created", json!({"info":info}));
        created["location"] =
            json!({"directory":directory.to_string_lossy().trim_start_matches(r"\\?\")});
        let _ = self.events.send(created);
        let output = pty.clone();
        std::thread::spawn(move || {
            let mut chunk = [0_u8; 8192];
            let mut pending = Vec::new();
            while let Ok(count) = reader.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                pending.extend_from_slice(&chunk[..count]);
                loop {
                    match std::str::from_utf8(&pending) {
                        Ok(text) => {
                            output.push(text.to_owned());
                            pending.clear();
                            break;
                        }
                        Err(error) => {
                            let valid = error.valid_up_to();
                            if valid > 0 {
                                output
                                    .push(String::from_utf8_lossy(&pending[..valid]).into_owned());
                                pending.drain(..valid);
                            }
                            match error.error_len() {
                                Some(count) => {
                                    output.push("�".into());
                                    pending.drain(..count);
                                }
                                None => break,
                            }
                        }
                    }
                }
            }
            if !pending.is_empty() {
                output.push(String::from_utf8_lossy(&pending).into_owned());
            }
            output
                .reader_done
                .store(true, std::sync::atomic::Ordering::Release);
        });
        let events = self.events.clone();
        std::thread::spawn(move || {
            let exit = child.wait().map(|status| status.exit_code()).unwrap_or(1);
            pty.info.lock().unwrap()["status"] = json!("exited");
            pty.info.lock().unwrap()["exitCode"] = json!(exit);
            let mut exited =
                crate::desktop_api::event("pty.exited", json!({"id":id,"exitCode":exit}));
            exited["location"] =
                json!({"directory":pty.directory.to_string_lossy().trim_start_matches(r"\\?\")});
            let _ = events.send(exited);
        });
        Ok(info)
    }
    pub fn get(&self, id: &str, directory: &Path) -> Result<Arc<Pty>> {
        let pty = self
            .sessions
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .context("terminal not found")?;
        ensure!(
            pty.directory == directory.canonicalize()?,
            "terminal belongs to another project"
        );
        Ok(pty)
    }
    pub fn list(&self, directory: &Path) -> Result<Vec<Value>> {
        let directory = directory.canonicalize()?;
        Ok(self
            .sessions
            .lock()
            .unwrap()
            .values()
            .filter(|pty| pty.directory == directory)
            .map(|pty| pty.info())
            .collect())
    }
    pub fn remove(&self, id: &str, directory: &Path) -> Result<()> {
        self.get(id, directory)?.kill()?;
        self.sessions.lock().unwrap().remove(id);
        let mut deleted = crate::desktop_api::event("pty.deleted", json!({"id":id}));
        deleted["location"] = json!({"directory":directory.canonicalize()?.to_string_lossy().trim_start_matches(r"\\?\")});
        let _ = self.events.send(deleted);
        Ok(())
    }
    pub fn ticket(&self, id: &str, directory: &Path) -> Result<String> {
        self.get(id, directory)?;
        let mut tickets = self.tickets.lock().unwrap();
        tickets.retain(|_, ticket| ticket.expires > Instant::now());
        ensure!(tickets.len() < 512, "too many outstanding terminal tickets");
        let token = Uuid::new_v4().to_string();
        tickets.insert(
            token.clone(),
            Ticket {
                pty: id.to_owned(),
                directory: directory.canonicalize()?,
                expires: Instant::now() + Duration::from_secs(60),
            },
        );
        Ok(token)
    }
    pub fn connect(&self, id: &str, directory: &Path, token: &str) -> Result<Arc<Pty>> {
        let ticket = self
            .tickets
            .lock()
            .unwrap()
            .remove(token)
            .context("invalid or consumed terminal ticket")?;
        ensure!(
            ticket.expires > Instant::now()
                && ticket.pty == id
                && ticket.directory == directory.canonicalize()?,
            "invalid terminal ticket scope"
        );
        self.get(id, directory)
    }
}
impl Drop for PtyManager {
    fn drop(&mut self) {
        for pty in self.sessions.lock().unwrap().values() {
            let _ = pty.kill();
        }
    }
}
impl Pty {
    pub fn drained(&self) -> bool {
        self.reader_done.load(std::sync::atomic::Ordering::Acquire)
            && self.info()["status"] == "exited"
    }
    pub fn info(&self) -> Value {
        self.info.lock().unwrap().clone()
    }
    fn push(&self, text: String) {
        if text.is_empty() {
            return;
        }
        let mut buffer = self.buffer.lock().unwrap();
        let output = Output {
            start: buffer.cursor,
            text,
        };
        buffer.cursor += output.text.encode_utf16().count() as u64;
        buffer.bytes += output.text.len();
        buffer.chunks.push_back(output.clone());
        while buffer.bytes > 8_000_000 {
            if let Some(old) = buffer.chunks.pop_front() {
                buffer.bytes -= old.text.len();
            }
        }
        let _ = self.output.send(output);
    }
    pub fn attach(
        &self,
        cursor: Option<u64>,
    ) -> Result<(Vec<Output>, u64, broadcast::Receiver<Output>)> {
        let buffer = self.buffer.lock().unwrap();
        let start = buffer
            .chunks
            .front()
            .map(|chunk| chunk.start)
            .unwrap_or(buffer.cursor);
        let cursor = cursor.unwrap_or(start);
        ensure!(
            cursor >= start && cursor <= buffer.cursor,
            "terminal output cursor is no longer available"
        );
        let chunks = buffer
            .chunks
            .iter()
            .filter_map(|chunk| {
                let end = chunk.start + chunk.text.encode_utf16().count() as u64;
                if end <= cursor {
                    return None;
                }
                let skip = cursor.saturating_sub(chunk.start) as usize;
                Some(Output {
                    start: chunk.start.max(cursor),
                    text: String::from_utf16_lossy(
                        &chunk.text.encode_utf16().skip(skip).collect::<Vec<_>>(),
                    ),
                })
            })
            .collect();
        Ok((chunks, buffer.cursor, self.output.subscribe()))
    }
    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        ensure!(bytes.len() <= 65536, "terminal input is too large");
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }
    pub fn update(&self, input: &Value) -> Result<Value> {
        if let Some(size) = input.get("size") {
            let rows = size["rows"].as_u64().context("invalid terminal rows")?;
            let cols = size["cols"].as_u64().context("invalid terminal columns")?;
            ensure!(
                (1..=1000).contains(&rows) && (1..=1000).contains(&cols),
                "terminal size out of range"
            );
            self.master.lock().unwrap().resize(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        }
        if let Some(title) = input["title"].as_str() {
            self.info.lock().unwrap()["title"] = json!(title);
        }
        Ok(self.info())
    }
    pub fn kill(&self) -> Result<()> {
        if self.info.lock().unwrap()["status"] != "exited" {
            self.killer.lock().unwrap().kill()?;
        }
        Ok(())
    }
}
