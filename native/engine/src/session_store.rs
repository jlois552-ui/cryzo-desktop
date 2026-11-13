use crate::store::Store;
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let current = now() << 12;
    let mut previous = LAST.load(Ordering::SeqCst);
    let stamp = loop {
        let next = current.max(previous + 1);
        match LAST.compare_exchange_weak(previous, next, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break next & 0xffffffffffff,
            Err(value) => previous = value,
        }
    };
    format!(
        "{prefix}_{stamp:012x}{}",
        &uuid::Uuid::new_v4().simple().to_string()[..14]
    )
}
pub fn tokens() -> Value {
    json!({"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}})
}

impl Store {
    pub fn desktop_states(&self) -> Result<Vec<Value>> {
        let db = self.0.lock().unwrap();
        let mut query = db.prepare("SELECT data FROM desktop_state ORDER BY rowid DESC")?;
        query
            .query_map([], |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect()
    }
    pub fn desktop_state(&self, id: &str) -> Result<Value> {
        let data: String = self
            .0
            .lock()
            .unwrap()
            .query_row("SELECT data FROM desktop_state WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .context("session not found")?;
        Ok(serde_json::from_str(&data)?)
    }
    pub fn desktop_create(&self, info: Value) -> Result<Value> {
        let id = info["id"].as_str().context("session ID missing")?;
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        ensure!(
            tx.query_row("SELECT 1 FROM desktop_state WHERE id=?1", [id], |_| Ok(()))
                .optional()?
                .is_none(),
            "session already exists"
        );
        tx.execute(
            "INSERT INTO sessions(id,directory,title) VALUES(?1,?2,?3)",
            params![
                id,
                info["location"]["directory"].as_str(),
                info["title"].as_str().unwrap_or("New session")
            ],
        )?;
        let mut data = info.clone();
        data["sessionID"] = json!(id);
        data["slug"] = json!(id);
        data["version"] = json!(env!("CARGO_PKG_VERSION"));
        for key in ["id", "time", "cost", "tokens", "outcome"] {
            data.as_object_mut().unwrap().remove(key);
        }
        let event = log(&tx, id, "session.created", data)?;
        let state = json!({"info":info,"messages":[],"inbox":[],"forms":[],"status":"idle"});
        tx.execute(
            "INSERT INTO desktop_state(id,data) VALUES(?1,?2)",
            params![id, state.to_string()],
        )?;
        tx.commit()?;
        Ok(event)
    }
    pub fn desktop_append(&self, id: &str, kind: &str, mut data: Value) -> Result<Value> {
        let mut db = self.0.lock().unwrap();
        let tx = db.transaction()?;
        if kind == "session.inbox.enqueued" {
            let input = data["inboxID"].as_str().context("prompt ID missing")?;
            let owner: Option<String> = tx
                .query_row(
                    "SELECT session_id FROM desktop_prompt_owners WHERE id=?1",
                    [input],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(owner) = owner {
                ensure!(owner == id, "prompt ID belongs to another session");
                let previous:String=tx.query_row("SELECT data FROM desktop_log WHERE session_id=?1 AND json_extract(data,'$.type')='session.inbox.enqueued' AND json_extract(data,'$.data.inboxID')=?2",params![id,input],|r|r.get(0))?;
                return Ok(serde_json::from_str(&previous)?);
            }
            tx.execute(
                "INSERT INTO desktop_prompt_owners(id,session_id) VALUES(?1,?2)",
                params![input, id],
            )?;
        }
        let raw: String = tx
            .query_row("SELECT data FROM desktop_state WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .context("session not found")?;
        let mut state: Value = serde_json::from_str(&raw)?;
        data["sessionID"] = json!(id);
        let event = log(&tx, id, kind, data)?;
        project(&mut state, &event)?;
        tx.execute(
            "UPDATE desktop_state SET data=?2 WHERE id=?1",
            params![id, state.to_string()],
        )?;
        tx.execute(
            "UPDATE sessions SET title=?2,status=?3 WHERE id=?1",
            params![
                id,
                state["info"]["title"].as_str().unwrap_or("New session"),
                state["status"].as_str()
            ],
        )?;
        tx.commit()?;
        Ok(event)
    }
    pub fn desktop_log(&self, id: &str, after: u64) -> Result<Vec<Value>> {
        let db = self.0.lock().unwrap();
        let mut query = db.prepare(
            "SELECT data FROM desktop_log WHERE session_id=?1 AND seq>?2 ORDER BY seq LIMIT 500",
        )?;
        query
            .query_map(params![id, after], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect()
    }
    pub fn desktop_meta(&self, namespace: &str) -> Result<Vec<Value>> {
        let db = self.0.lock().unwrap();
        let mut query = db.prepare("SELECT data FROM desktop_meta WHERE namespace=?1")?;
        query
            .query_map([namespace], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect()
    }
    pub fn desktop_put_meta(&self, namespace: &str, key: &str, data: Option<&Value>) -> Result<()> {
        let db = self.0.lock().unwrap();
        if let Some(data) = data {
            db.execute("INSERT INTO desktop_meta(namespace,key,data) VALUES(?1,?2,?3) ON CONFLICT(namespace,key) DO UPDATE SET data=excluded.data",params![namespace,key,data.to_string()])?;
        } else {
            db.execute(
                "DELETE FROM desktop_meta WHERE namespace=?1 AND key=?2",
                params![namespace, key],
            )?;
        }
        Ok(())
    }
}
fn log(tx: &Transaction<'_>, id: &str, kind: &str, data: Value) -> Result<Value> {
    let seq: u64 = tx.query_row(
        "SELECT COALESCE(MAX(seq),0)+1 FROM desktop_log WHERE session_id=?1",
        [id],
        |r| r.get(0),
    )?;
    let version = if matches!(
        kind,
        "session.tool.success" | "session.tool.failed" | "session.deleted"
    ) {
        2
    } else {
        1
    };
    let event = json!({"id":self::id("evt"),"created":now(),"type":kind,"data":data,"durable":{"aggregateID":id,"seq":seq,"version":version}});
    tx.execute(
        "INSERT INTO desktop_log(session_id,seq,data) VALUES(?1,?2,?3)",
        params![id, seq, event.to_string()],
    )?;
    Ok(event)
}
fn project(state: &mut Value, event: &Value) -> Result<()> {
    let data = &event["data"];
    let at = &event["created"];
    state["info"]["time"]["updated"] = at.clone();
    match event["type"].as_str().unwrap_or_default() {
        "session.renamed"=>state["info"]["title"]=data["title"].clone(),
        "session.model.selected"=>{state["info"]["model"]=data["model"].clone();state["messages"].as_array_mut().unwrap().push(json!({"id":event["id"].as_str().unwrap().replacen("evt_","msg_",1),"type":"model-switched","model":data["model"],"time":{"created":at}}));},
        "session.agent.selected"=>{state["info"]["agent"]=data["agent"].clone();state["messages"].as_array_mut().unwrap().push(json!({"id":event["id"].as_str().unwrap().replacen("evt_","msg_",1),"type":"agent-switched","agent":data["agent"],"time":{"created":at}}));},
        "session.viewed"=>state["info"]["time"]["viewed"]=data["idle"].clone(),
        "session.archived"=>state["info"]["time"]["archived"]=at.clone(),
        "session.unarchived"=>{state["info"]["time"].as_object_mut().unwrap().remove("archived");},
        "session.deleted"=>state["deleted"]=json!(true),
        "session.inbox.enqueued"=>{
            let inbox=data["inboxID"].as_str().context("missing inbox ID")?;
            ensure!(!state["inbox"].as_array().unwrap().iter().any(|item|item["id"]==inbox) && !state["messages"].as_array().unwrap().iter().any(|item|item["id"]==inbox),"duplicate prompt ID");
            let mut item=data["item"].clone(); item["id"]=json!(inbox);item["sessionID"]=data["sessionID"].clone();item["time"]=json!({"created":at});
            state["inbox"].as_array_mut().unwrap().push(item);
        }
        "session.inbox.delivered"=>{
            let inbox=state["inbox"].as_array_mut().unwrap();
            let index=inbox.iter().position(|item|item["id"]==data["inboxID"]).context("inbox item missing")?;
            let item=inbox.remove(index); let mut message=item["payload"].clone();
            message["id"]=item["id"].clone();message["type"]=item["type"].clone();message["time"]=item["time"].clone();
            state["messages"].as_array_mut().unwrap().push(message);
        }
        "session.inbox.cancelled"=>state["inbox"].as_array_mut().unwrap().retain(|item|item["id"]!=data["inboxID"]),
        "session.execution.started"=>{state["status"]=json!("running");state["info"].as_object_mut().unwrap().remove("outcome");},
        "session.execution.succeeded"|"session.execution.failed"|"session.execution.interrupted"=>{
            let outcome=event["type"].as_str().unwrap().rsplit('.').next().unwrap();
            state["status"]=json!("idle");state["info"]["outcome"]=json!(outcome);state["info"]["time"]["idle"]=at.clone();
            if data["reason"] != "shutdown" {
                let mut message = json!({"id":event["id"].as_str().unwrap().replacen("evt_","msg_",1),"type":"idle","outcome":outcome,"time":{"created":at}});
                if let Some(error) = data.get("error") { message["error"]=error.clone(); }
                state["messages"].as_array_mut().unwrap().push(message);
            }
            for form in state["forms"].as_array_mut().unwrap() { if form["state"]["status"]=="pending" {form["state"]=json!({"status":"cancelled"});} }
        }
        "session.step.started"=>state["messages"].as_array_mut().unwrap().push(json!({"id":data["assistantMessageID"],"type":"assistant","agent":data["agent"],"model":data["model"],"content":[],"time":{"created":at}})),
        kind if kind.starts_with("session.reasoning.") || kind.starts_with("session.text.") || kind.starts_with("session.tool.") || kind.starts_with("session.step.")=>{
            let message=state["messages"].as_array_mut().unwrap().iter_mut().find(|message|message["id"]==data["assistantMessageID"]).context("assistant message missing")?;
            match kind {
                "session.reasoning.started"=>message["content"].as_array_mut().unwrap().push(json!({"type":"reasoning","text":""})),
                "session.reasoning.ended"=>{let index=data["ordinal"].as_u64().context("reasoning ordinal missing")? as usize;message["content"][index]=json!({"type":"reasoning","text":data["text"],"native":data.get("native").filter(|v|v.is_object()).cloned().unwrap_or(json!({}))});},
                "session.text.started"=>message["content"].as_array_mut().unwrap().push(json!({"type":"text","text":""})),
                "session.text.ended"=>{let index=data["ordinal"].as_u64().context("text ordinal missing")? as usize;message["content"][index]=json!({"type":"text","text":data["text"],"native":data.get("native").filter(|v|v.is_object()).cloned().unwrap_or(json!({}))});},
                "session.step.streamed"=>message["time"]["streamed"]=at.clone(),
                "session.step.ended"=>{
                    message["finish"]=data["finish"].clone();message["time"]["completed"]=at.clone();message["tokens"]=data["tokens"].clone();message["cost"]=data["cost"].clone();
                    state["info"]["cost"]=json!(state["info"]["cost"].as_f64().unwrap_or(0.0)+data["cost"].as_f64().unwrap_or(0.0));
                    for key in ["input","output","reasoning"] {state["info"]["tokens"][key]=json!(state["info"]["tokens"][key].as_f64().unwrap_or(0.0)+data["tokens"][key].as_f64().unwrap_or(0.0));}
                    for key in ["read","write"] {state["info"]["tokens"]["cache"][key]=json!(state["info"]["tokens"]["cache"][key].as_f64().unwrap_or(0.0)+data["tokens"]["cache"][key].as_f64().unwrap_or(0.0));}
                }
                "session.step.failed"=>{message["error"]=data["error"].clone();message["time"]["completed"]=at.clone();},
                "session.tool.input.started"=>message["content"].as_array_mut().unwrap().push(json!({"type":"tool","native":data.get("native").filter(|v|v.is_object()).cloned().unwrap_or(json!({})),"id":data["id"],"name":data["name"],"state":{"status":"streaming","input":""},"time":{"created":at}})),
                _=>{
                    let tool=message["content"].as_array_mut().unwrap().iter_mut().find(|tool|tool["type"]=="tool" && tool["id"]==data["id"]).context("tool call missing")?;
                    match kind {
                        "session.tool.input.ended"=>tool["state"]["input"]=data["text"].clone(),
                        "session.tool.called"=>{tool["state"]=json!({"status":"running","input":data["input"],"metadata":{}});tool["time"]["ran"]=at.clone();},
                        "session.tool.success"=>{tool["state"]=json!({"status":"completed","input":tool["state"]["input"],"content":data["content"]});tool["time"]["completed"]=at.clone();},
                        "session.tool.failed"=>{tool["state"]=json!({"status":"error","input":tool["state"]["input"],"error":data["error"]});tool["time"]["completed"]=at.clone();},
                        _=>anyhow::bail!("unsupported session projection: {kind}"),
                    }
                }
            }
        }
        _=>anyhow::bail!("unsupported session event"),
    }
    Ok(())
}
