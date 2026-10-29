use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::Path, sync::Mutex};
use uuid::Uuid;

pub struct Store(pub(crate) Mutex<Connection>);

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub directory: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Event {
    pub sequence: i64,
    pub session_id: String,
    pub kind: String,
    pub data: Value,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS sessions (
               id TEXT PRIMARY KEY, directory TEXT NOT NULL, title TEXT NOT NULL,
               status TEXT NOT NULL DEFAULT 'idle');
             CREATE TABLE IF NOT EXISTS events (
               sequence INTEGER PRIMARY KEY AUTOINCREMENT,
               session_id TEXT NOT NULL REFERENCES sessions(id),
               kind TEXT NOT NULL, data TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS events_session ON events(session_id, sequence);
             CREATE TABLE IF NOT EXISTS inbox (
               id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id),
               payload TEXT NOT NULL, delivered INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS approvals (
               id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id),
               operation TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending');
             CREATE TABLE IF NOT EXISTS desktop_projects (
               id TEXT PRIMARY KEY, directory TEXT NOT NULL UNIQUE, info TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS desktop_state (id TEXT PRIMARY KEY, data TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS desktop_log (session_id TEXT NOT NULL, seq INTEGER NOT NULL, data TEXT NOT NULL, PRIMARY KEY(session_id,seq));
             CREATE TABLE IF NOT EXISTS desktop_meta (namespace TEXT NOT NULL, key TEXT NOT NULL, data TEXT NOT NULL, PRIMARY KEY(namespace,key));
             CREATE TABLE IF NOT EXISTS desktop_prompt_owners (id TEXT PRIMARY KEY, session_id TEXT NOT NULL);
             PRAGMA user_version=1;",
        )?;
        let store = Self(Mutex::new(connection));
        // A process may have died after a tool was started. Do not replay effects automatically.
        store.0.lock().unwrap().execute(
            "UPDATE sessions SET status='interrupted' WHERE status='running'",
            [],
        )?;
        for state in store.desktop_states()? {
            if state["status"] == "running" {
                let id = state["info"]["id"].as_str().unwrap();
                store.desktop_append(
                    id,
                    "session.execution.interrupted",
                    serde_json::json!({"reason":"shutdown"}),
                )?;
            }
        }
        Ok(store)
    }

    pub fn desktop_project(&self, directory: &Path) -> Result<Value> {
        let directory = directory.canonicalize()?;
        ensure!(directory.is_dir(), "project location must be a directory");
        let directory = directory
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_owned();
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT info FROM desktop_projects WHERE directory=?1",
                [&directory],
                |row| row.get(0),
            )
            .optional()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;
        let mut info = match existing {
            Some(info) => serde_json::from_str::<Value>(&info)?,
            None => {
                let mut info = serde_json::json!({
                    "id":format!("prj_{}", Uuid::new_v4()),"canonical":directory,
                    "time":{"created":now,"updated":now,"active":now},"sandboxes":[]
                });
                if Path::new(&directory).join(".git").exists() {
                    info["vcs"] = Value::String("git".into());
                }
                info
            }
        };
        info["time"]["active"] = serde_json::json!(now);
        tx.execute("INSERT INTO desktop_projects(id,directory,info) VALUES(?1,?2,?3) ON CONFLICT(directory) DO UPDATE SET info=excluded.info", params![info["id"].as_str(),directory,info.to_string()])?;
        tx.commit()?;
        Ok(info)
    }

    pub fn desktop_projects(&self) -> Result<Vec<Value>> {
        let db = self.0.lock().unwrap();
        let mut query = db.prepare(
            "SELECT info FROM desktop_projects ORDER BY json_extract(info,'$.time.active') DESC",
        )?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|row| Ok(serde_json::from_str(&row)?))
            .collect()
    }

    pub fn create_session(&self, directory: &Path, title: &str) -> Result<Session> {
        let session = Session {
            id: Uuid::new_v4().to_string(),
            directory: directory.canonicalize()?.to_string_lossy().into_owned(),
            title: title.to_owned(),
            status: "idle".into(),
        };
        self.0.lock().unwrap().execute(
            "INSERT INTO sessions(id,directory,title) VALUES (?1,?2,?3)",
            params![session.id, session.directory, session.title],
        )?;
        Ok(session)
    }

    pub fn session(&self, id: &str) -> Result<Session> {
        Ok(self.0.lock().unwrap().query_row(
            "SELECT id,directory,title,status FROM sessions WHERE id=?1",
            [id],
            |row| {
                Ok(Session {
                    id: row.get(0)?,
                    directory: row.get(1)?,
                    title: row.get(2)?,
                    status: row.get(3)?,
                })
            },
        )?)
    }

    pub fn sessions(&self) -> Result<Vec<Session>> {
        let connection = self.0.lock().unwrap();
        let mut query = connection.prepare(
            "SELECT id,directory,title,status FROM sessions ORDER BY rowid DESC LIMIT 100",
        )?;
        Ok(query
            .query_map([], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    directory: row.get(1)?,
                    title: row.get(2)?,
                    status: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// First admission wins even after delivery; an ID cannot cross session boundaries.
    pub fn admit(&self, session: &str, id: &str, payload: &Value) -> Result<bool> {
        let mut connection = self.0.lock().unwrap();
        let transaction = connection.transaction()?;
        let owner: Option<String> = transaction
            .query_row("SELECT session_id FROM inbox WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(owner) = owner {
            ensure!(
                owner == session,
                "message ID belongs to a different session"
            );
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO inbox(id,session_id,payload) VALUES(?1,?2,?3)",
            params![id, session, payload.to_string()],
        )?;
        transaction.execute(
            "INSERT INTO events(session_id,kind,data) VALUES(?1,'prompt.admitted',?2)",
            params![session, serde_json::json!({"id":id}).to_string()],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn deliver(&self, session: &str, id: &str) -> Result<Option<Value>> {
        let mut connection = self.0.lock().unwrap();
        let transaction = connection.transaction()?;
        let payload: Option<String> = transaction
            .query_row(
                "SELECT payload FROM inbox WHERE session_id=?1 AND id=?2 AND delivered=0",
                params![session, id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        transaction.execute("UPDATE inbox SET delivered=1 WHERE id=?1", [id])?;
        transaction.execute(
            "INSERT INTO events(session_id,kind,data) VALUES(?1,'message.user',?2)",
            params![session, payload],
        )?;
        transaction.commit()?;
        Ok(Some(serde_json::from_str(&payload)?))
    }

    pub fn append(&self, session: &str, kind: &str, data: &Value) -> Result<i64> {
        let connection = self.0.lock().unwrap();
        connection.execute(
            "INSERT INTO events(session_id,kind,data) VALUES(?1,?2,?3)",
            params![session, kind, data.to_string()],
        )?;
        Ok(connection.last_insert_rowid())
    }

    pub fn events(&self, session: &str, after: i64) -> Result<Vec<Event>> {
        let connection = self.0.lock().unwrap();
        let mut query = connection.prepare("SELECT sequence,session_id,kind,data FROM events WHERE session_id=?1 AND sequence>?2 ORDER BY sequence LIMIT 500")?;
        let rows = query.query_map(params![session, after], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (sequence, session_id, kind, data) = row?;
            Ok(Event {
                sequence,
                session_id,
                kind,
                data: serde_json::from_str(&data)?,
            })
        })
        .collect()
    }

    pub fn claim(&self, session: &str) -> Result<()> {
        let changed = self.0.lock().unwrap().execute(
            "UPDATE sessions SET status='running' WHERE id=?1 AND status!='running'",
            [session],
        )?;
        ensure!(changed == 1, "session is already running or does not exist");
        Ok(())
    }

    pub fn finish(&self, session: &str, status: &str) -> Result<()> {
        ensure!(
            ["idle", "interrupted", "failed"].contains(&status),
            "invalid status"
        );
        self.0.lock().unwrap().execute(
            "UPDATE sessions SET status=?2 WHERE id=?1",
            params![session, status],
        )?;
        Ok(())
    }

    pub fn request_approval(&self, session: &str, operation: &Value) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        self.0.lock().unwrap().execute(
            "INSERT INTO approvals(id,session_id,operation) VALUES(?1,?2,?3)",
            params![id, session, operation.to_string()],
        )?;
        Ok(id)
    }

    pub fn decide(&self, session: &str, id: &str, allow: bool) -> Result<()> {
        let changed = self.0.lock().unwrap().execute(
            "UPDATE approvals SET status=?3 WHERE id=?1 AND session_id=?2 AND status='pending'",
            params![id, session, if allow { "allowed" } else { "denied" }],
        )?;
        ensure!(changed == 1, "approval is not pending for this session");
        Ok(())
    }

    pub fn consume_approval(&self, session: &str, id: &str, operation: &Value) -> Result<()> {
        let changed = self.0.lock().unwrap().execute("UPDATE approvals SET status='consumed' WHERE id=?1 AND session_id=?2 AND operation=?3 AND status='allowed'", params![id,session,operation.to_string()])?;
        ensure!(changed == 1, "exact operation requires an unused approval");
        Ok(())
    }
}
