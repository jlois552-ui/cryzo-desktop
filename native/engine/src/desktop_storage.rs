//! Durable implementation of the frozen desktop storage and draft contracts.
//! Credentials belong in the OS keyring, never in this store.
use anyhow::{Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path, sync::Mutex};
use uuid::Uuid;

pub struct DesktopStorage(Mutex<Connection>);

impl DesktopStorage {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS namespaces(name TEXT PRIMARY KEY, revision INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS items(namespace TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
               PRIMARY KEY(namespace,key));
             CREATE TABLE IF NOT EXISTS drafts(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS blobs(id TEXT PRIMARY KEY, data BLOB NOT NULL);",
        )?;
        Ok(Self(Mutex::new(db)))
    }

    pub fn call(&self, tag: &str, payload: &Value) -> Result<Value> {
        let mut db = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("desktop storage lock poisoned"))?;
        let text = |key: &str| -> Result<String> {
            let value = payload[key]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing {key}"))?;
            ensure!(value.len() <= 4_000_000, "desktop value is too large");
            Ok(value.to_owned())
        };
        match tag {
            "StorageItems" => {
                let name = text("name")?;
                let revision: i64 = db
                    .query_row(
                        "SELECT revision FROM namespaces WHERE name=?1",
                        [&name],
                        |r| r.get(0),
                    )
                    .optional()?
                    .unwrap_or(0);
                let mut query = db.prepare("SELECT key,value FROM items WHERE namespace=?1")?;
                let items = query
                    .query_map([name], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            Value::String(r.get::<_, String>(1)?),
                        ))
                    })?
                    .collect::<rusqlite::Result<serde_json::Map<String, Value>>>()?;
                Ok(json!({"items":items,"revision":revision}))
            }
            "StorageUpdate" | "StorageClear" => {
                let name = text("name")?;
                let tx = db.transaction()?;
                tx.execute("INSERT INTO namespaces(name,revision) VALUES(?1,1) ON CONFLICT(name) DO UPDATE SET revision=revision+1", [&name])?;
                if tag == "StorageClear" {
                    tx.execute("DELETE FROM items WHERE namespace=?1", [&name])?;
                } else {
                    let insert = payload["insert"]
                        .as_object()
                        .ok_or_else(|| anyhow::anyhow!("invalid insert"))?;
                    for (key, value) in insert {
                        let value = value
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("storage values must be strings"))?;
                        ensure!(value.len() <= 4_000_000, "storage value too large");
                        tx.execute("INSERT INTO items(namespace,key,value) VALUES(?1,?2,?3) ON CONFLICT(namespace,key) DO UPDATE SET value=excluded.value", params![name,key,value])?;
                    }
                    for key in payload["remove"]
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("invalid remove"))?
                    {
                        let key = key
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("invalid storage key"))?;
                        tx.execute(
                            "DELETE FROM items WHERE namespace=?1 AND key=?2",
                            params![name, key],
                        )?;
                    }
                }
                let revision: i64 = tx.query_row(
                    "SELECT revision FROM namespaces WHERE name=?1",
                    [name],
                    |r| r.get(0),
                )?;
                tx.commit()?;
                Ok(if tag == "StorageClear" {
                    Value::Null
                } else {
                    json!(revision)
                })
            }
            "DraftsGet" => Ok(db
                .query_row(
                    "SELECT value FROM drafts WHERE key=?1",
                    [text("key")?],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .map(Value::String)
                .unwrap_or(Value::Null)),
            "DraftsDelete" => {
                db.execute("DELETE FROM drafts WHERE key=?1", [text("key")?])?;
                Ok(Value::Null)
            }
            "DraftsSet" => {
                let key = text("key")?;
                let value = text("value")?;
                let mut references = BTreeSet::new();
                if let Ok(document) = serde_json::from_str::<Value>(&value) {
                    blob_references(&document, &mut references);
                }
                let missing = references
                    .into_iter()
                    .filter_map(|id| {
                        match db
                            .query_row("SELECT 1 FROM blobs WHERE id=?1", [&id], |_| Ok(()))
                            .optional()
                        {
                            Ok(Some(())) => None,
                            Ok(None) => Some(Ok(id)),
                            Err(error) => Some(Err(error)),
                        }
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                if !payload["strict"].as_bool().unwrap_or(false) || missing.is_empty() {
                    db.execute("INSERT INTO drafts(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value])?;
                }
                Ok(json!(missing))
            }
            "DraftsPutBlob" => {
                let bytes: Vec<u8> = serde_json::from_value(payload["data"].clone())?;
                ensure!(bytes.len() <= 20_000_000, "draft attachment exceeds 20 MB");
                let used: i64 =
                    db.query_row("SELECT COALESCE(SUM(length(data)),0) FROM blobs", [], |r| {
                        r.get(0)
                    })?;
                ensure!(
                    used + bytes.len() as i64 <= 512_000_000,
                    "draft storage is full"
                );
                let id = Uuid::new_v4().to_string();
                db.execute(
                    "INSERT INTO blobs(id,data) VALUES(?1,?2)",
                    params![id, bytes],
                )?;
                Ok(json!(id))
            }
            "DraftsGetBlob" => Ok(db
                .query_row("SELECT data FROM blobs WHERE id=?1", [text("id")?], |r| {
                    r.get::<_, Vec<u8>>(0)
                })
                .optional()?
                .map(|bytes| json!(bytes))
                .unwrap_or(Value::Null)),
            _ => bail!("unsupported desktop storage operation: {tag}"),
        }
    }
}

fn blob_references(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(blob) = object.get("blob") {
                if let Some(id) = blob["id"].as_str() {
                    ids.insert(id.to_owned());
                }
                if let Some(chunks) = blob["ids"].as_array() {
                    ids.extend(chunks.iter().filter_map(Value::as_str).map(str::to_owned));
                }
            }
            for child in object.values() {
                blob_references(child, ids);
            }
        }
        Value::Array(array) => {
            for child in array {
                blob_references(child, ids);
            }
        }
        _ => (),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_survive_restart_and_failed_updates_are_atomic() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("desktop.sqlite");
        let store = DesktopStorage::open(&path)?;
        assert_eq!(
            store.call(
                "StorageUpdate",
                &json!({"name":"layout","insert":{"theme":"dark"},"remove":[]})
            )?,
            json!(1)
        );
        assert!(
            store
                .call(
                    "StorageUpdate",
                    &json!({"name":"layout","insert":{"bad":false},"remove":[]})
                )
                .is_err()
        );
        drop(store);
        let store = DesktopStorage::open(&path)?;
        assert_eq!(
            store.call("StorageItems", &json!({"name":"layout"}))?,
            json!({"items":{"theme":"dark"},"revision":1})
        );
        assert_eq!(
            store.call("StorageItems", &json!({"name":"other"}))?,
            json!({"items":{},"revision":0})
        );
        Ok(())
    }

    #[test]
    fn strict_drafts_keep_previous_document_until_attachments_exist() -> Result<()> {
        let store = DesktopStorage::open(Path::new(":memory:"))?;
        store.call(
            "DraftsSet",
            &json!({"key":"composer","value":"previous","strict":true}),
        )?;
        let draft = json!({"parts":[{"blob":{"id":"missing"}}]}).to_string();
        assert_eq!(
            store.call(
                "DraftsSet",
                &json!({"key":"composer","value":draft,"strict":true})
            )?,
            json!(["missing"])
        );
        assert_eq!(
            store.call("DraftsGet", &json!({"key":"composer"}))?,
            json!("previous")
        );
        let id = store.call("DraftsPutBlob", &json!({"data":[0,127,255]}))?;
        assert_eq!(
            store.call("DraftsGetBlob", &json!({"id":id}))?,
            json!([0, 127, 255])
        );
        Ok(())
    }
}
