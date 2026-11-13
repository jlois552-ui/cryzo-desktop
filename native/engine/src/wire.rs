//! Native wire adapters. The agent's canonical conversation remains provider-independent.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn prepare(protocol: &str, mut request: Value) -> Result<(String, Value)> {
    let messages = request["messages"].as_array().context("messages missing")?;
    if protocol == "chat-completions" {
        request["stream"] = json!(true);
        return Ok(("chat/completions".into(), request));
    }
    if protocol == "responses" {
        let mut input = Vec::new();
        for message in messages {
            if message["role"] == "tool" {
                input.push(json!({"type":"function_call_output","call_id":message["tool_call_id"],"output":message["content"]}));
                continue;
            }
            if let Some(native) = message["cryzo_native"]["responses"].as_array() {
                input.extend(native.iter().cloned());
            }
            let content = message["content"].as_str().unwrap_or("");
            if !content.is_empty() {
                input.push(json!({"role":message["role"],"content":content}));
            }
            if let Some(calls) = message["tool_calls"].as_array() {
                for call in calls {
                    input.push(json!({"type":"function_call","call_id":call["id"],"name":call["function"]["name"],"arguments":call["function"]["arguments"]}));
                }
            }
        }
        let mut body = json!({"model":request["model"],"input":input,"stream":true,"store":false,"include":["reasoning.encrypted_content"]});
        if let Some(tools) = request["tools"].as_array() {
            body["tools"]=json!(tools.iter().map(|t|json!({"type":"function","name":t["function"]["name"],"description":t["function"]["description"],"parameters":t["function"]["parameters"],"strict":false})).collect::<Vec<_>>());
        }
        if request["reasoning_effort"].is_string() {
            body["reasoning"] = json!({"effort":request["reasoning_effort"],"summary":"auto"});
        }
        return Ok(("responses".into(), body));
    }
    if protocol == "messages" {
        let mut system = Vec::new();
        let mut conversation = Vec::new();
        for message in messages {
            if message["role"] == "system" {
                system.push(json!({"type":"text","text":message["content"]}));
                continue;
            }
            if message["role"] == "tool" {
                append_turn(
                    &mut conversation,
                    "user",
                    "content",
                    vec![
                        json!({"type":"tool_result","tool_use_id":message["tool_call_id"],"content":message["content"]}),
                    ],
                );
                continue;
            }
            let mut content = message["cryzo_native"]["messages"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if let Some(text) = message["content"].as_str().filter(|t| !t.is_empty()) {
                content.push(json!({"type":"text","text":text}));
            }
            if let Some(calls) = message["tool_calls"].as_array() {
                for call in calls {
                    content.push(json!({"type":"tool_use","id":call["id"],"name":call["function"]["name"],"input":serde_json::from_str::<Value>(call["function"]["arguments"].as_str().context("tool arguments missing")?)?}));
                }
            }
            if !content.is_empty() {
                append_turn(
                    &mut conversation,
                    message["role"].as_str().context("role missing")?,
                    "content",
                    content,
                );
            }
        }
        let mut body = json!({"model":request["model"],"system":system,"messages":conversation,"max_tokens":16384,"stream":true});
        if let Some(tools) = request["tools"].as_array() {
            body["tools"]=json!(tools.iter().map(|t|json!({"name":t["function"]["name"],"description":t["function"]["description"],"input_schema":t["function"]["parameters"]})).collect::<Vec<_>>());
        }
        if let Some(effort) = request["reasoning_effort"]
            .as_str()
            .filter(|e| *e != "none")
        {
            body["thinking"] = json!({"type":"adaptive"});
            body["output_config"] = json!({"effort":effort});
        } else if let Some(enabled) = request
            .pointer("/chat_template_kwargs/enable_thinking")
            .and_then(Value::as_bool)
        {
            body["thinking"] = if enabled {
                json!({"type":"enabled","budget_tokens":8192})
            } else {
                json!({"type":"disabled"})
            };
        }
        return Ok(("messages".into(), body));
    }
    if protocol == "generate-content" {
        let mut system = Vec::new();
        let mut contents = Vec::new();
        let mut names = BTreeMap::new();
        for message in messages {
            if message["role"] == "system" {
                system.push(json!({"text":message["content"]}));
                continue;
            }
            if message["role"] == "tool" {
                let name = names
                    .get(message["tool_call_id"].as_str().unwrap_or(""))
                    .context("tool name missing from history")?;
                append_turn(
                    &mut contents,
                    "user",
                    "parts",
                    vec![
                        json!({"functionResponse":{"name":name,"response":{"output":message["content"]}}}),
                    ],
                );
                continue;
            }
            let mut parts = Vec::new();
            if let Some(text) = message["content"].as_str().filter(|t| !t.is_empty()) {
                parts.push(json!({"text":text}));
            }
            if let Some(calls) = message["tool_calls"].as_array() {
                for call in calls {
                    names.insert(
                        call["id"].as_str().unwrap_or("").to_owned(),
                        call["function"]["name"].clone(),
                    );
                }
            }
            if let Some(native) = message["cryzo_native"]["generate-content"].as_array() {
                parts = native.clone();
            } else if let Some(calls) = message["tool_calls"].as_array() {
                for call in calls {
                    parts.push(json!({"functionCall":{"name":call["function"]["name"],"args":serde_json::from_str::<Value>(call["function"]["arguments"].as_str().context("tool arguments missing")?)?}}));
                }
            }
            if !parts.is_empty() {
                append_turn(
                    &mut contents,
                    if message["role"] == "assistant" {
                        "model"
                    } else {
                        "user"
                    },
                    "parts",
                    parts,
                );
            }
        }
        let mut body = json!({"systemInstruction":{"parts":system},"contents":contents,"generationConfig":{"maxOutputTokens":16384}});
        if let Some(tools) = request["tools"].as_array() {
            body["tools"] = json!([{"functionDeclarations":tools.iter().map(|t|json!({"name":t["function"]["name"],"description":t["function"]["description"],"parametersJsonSchema":t["function"]["parameters"]})).collect::<Vec<_>>()}]);
        }
        if let Some(effort) = request["reasoning_effort"].as_str() {
            body["generationConfig"]["thinkingConfig"] =
                json!({"thinkingLevel":effort.to_uppercase(),"includeThoughts":true});
        } else if let Some(enabled) = request
            .pointer("/chat_template_kwargs/enable_thinking")
            .and_then(Value::as_bool)
        {
            body["generationConfig"]["thinkingConfig"] =
                json!({"thinkingBudget":if enabled{-1}else{0},"includeThoughts":enabled});
        }
        let model = request["model"].as_str().context("model missing")?;
        ensure!(
            !model.contains(['?', '#', '\\']) && !model.contains(".."),
            "invalid model ID"
        );
        return Ok((format!("models/{model}:streamGenerateContent"), body));
    }
    request
        .as_object_mut()
        .context("invalid request")?
        .remove("cryzo_native");
    anyhow::bail!("unsupported native transport")
}

fn append_turn(turns: &mut Vec<Value>, role: &str, field: &str, parts: Vec<Value>) {
    if let Some(last) = turns.last_mut().filter(|last| last["role"] == role) {
        last[field].as_array_mut().unwrap().extend(parts);
    } else {
        let mut turn = json!({"role":role});
        turn[field] = json!(parts);
        turns.push(turn);
    }
}

#[derive(Default)]
pub struct Decoder {
    usage: Value,
    native: Vec<Value>,
    thinking: BTreeMap<u64, Value>,
    details: BTreeMap<u64, Value>,
    calls: usize,
}
impl Decoder {
    pub fn native(&self, protocol: &str) -> Value {
        let mut native = json!({});
        native[protocol] = json!(self.native);
        native
    }
    pub fn event(&mut self, protocol: &str, event: Value) -> Result<Value> {
        if protocol == "chat-completions" {
            if let Some(parts) = event
                .pointer("/choices/0/delta/reasoning_details")
                .and_then(Value::as_array)
            {
                for (index, part) in parts.iter().enumerate() {
                    let index = part["index"].as_u64().unwrap_or(index as u64);
                    ensure!(index < 128, "too many reasoning blocks");
                    let target = self.details.entry(index).or_insert(json!({}));
                    for (key, value) in part.as_object().context("invalid reasoning block")? {
                        if ["text", "data", "signature"].contains(&key.as_str())
                            && value.is_string()
                        {
                            target[key] = json!(format!(
                                "{}{}",
                                target[key].as_str().unwrap_or(""),
                                value.as_str().unwrap()
                            ));
                        } else {
                            target[key] = value.clone();
                        }
                    }
                }
                self.native = self.details.values().cloned().collect();
                ensure!(
                    serde_json::to_string(&self.native)?.len() <= 4_000_000,
                    "provider replay data is too large"
                );
            }
            return Ok(event);
        }
        ensure!(
            event.get("error").is_none() && event["type"] != "error",
            "provider reported a generation error"
        );
        let mut delta = json!({});
        let mut finish = Value::Null;
        match protocol {
            "responses" => match event["type"].as_str().unwrap_or("") {
                "response.output_text.delta" => delta["content"] = event["delta"].clone(),
                "response.reasoning_summary_text.delta" => {
                    delta["reasoning_content"] = event["delta"].clone()
                }
                "response.output_item.added" if event["item"]["type"] == "function_call" => {
                    self.calls += 1;
                    delta["tool_calls"] = json!([{"index":event["output_index"],"id":event["item"]["call_id"],"function":{"name":event["item"]["name"],"arguments":""}}]);
                }
                "response.function_call_arguments.delta" => {
                    delta["tool_calls"] = json!([{"index":event["output_index"],"function":{"arguments":event["delta"]}}])
                }
                "response.output_item.done" if event["item"]["type"] == "reasoning" => {
                    self.native.push(event["item"].clone())
                }
                "response.completed" | "response.incomplete" => {
                    let u = &event["response"]["usage"];
                    self.usage = json!({"prompt_tokens":u["input_tokens"],"completion_tokens":u["output_tokens"],"prompt_tokens_details":u["input_tokens_details"],"completion_tokens_details":u["output_tokens_details"]});
                    finish = json!(if event["type"] == "response.incomplete" {
                        "length"
                    } else if self.calls > 0 {
                        "tool_calls"
                    } else {
                        "stop"
                    });
                }
                "response.failed" => anyhow::bail!("provider reported a generation error"),
                _ => {}
            },
            "messages" => match event["type"].as_str().unwrap_or("") {
                "message_start" => {
                    self.usage = json!({"prompt_tokens":event["message"]["usage"]["input_tokens"],"completion_tokens":0});
                }
                "content_block_start" => {
                    let block = &event["content_block"];
                    let index = event["index"].as_u64().context("block index missing")?;
                    ensure!(index < 64, "too many content blocks");
                    if block["type"] == "tool_use" {
                        self.calls += 1;
                        delta["tool_calls"] = json!([{"index":index,"id":block["id"],"function":{"name":block["name"],"arguments":""}}]);
                    }
                    if ["thinking", "redacted_thinking"]
                        .contains(&block["type"].as_str().unwrap_or(""))
                    {
                        self.thinking.insert(index, block.clone());
                    }
                }
                "content_block_delta" => {
                    let d = &event["delta"];
                    match d["type"].as_str().unwrap_or("") {
                        "text_delta" => delta["content"] = d["text"].clone(),
                        "thinking_delta" => {
                            delta["reasoning_content"] = d["thinking"].clone();
                            if let Some(block) =
                                self.thinking.get_mut(&event["index"].as_u64().unwrap_or(0))
                            {
                                let text = format!(
                                    "{}{}",
                                    block["thinking"].as_str().unwrap_or(""),
                                    d["thinking"].as_str().unwrap_or("")
                                );
                                block["thinking"] = json!(text);
                            }
                        }
                        "signature_delta" => {
                            if let Some(block) =
                                self.thinking.get_mut(&event["index"].as_u64().unwrap_or(0))
                            {
                                let signature = format!(
                                    "{}{}",
                                    block["signature"].as_str().unwrap_or(""),
                                    d["signature"].as_str().unwrap_or("")
                                );
                                block["signature"] = json!(signature);
                            }
                        }
                        "input_json_delta" => {
                            delta["tool_calls"] = json!([{"index":event["index"],"function":{"arguments":d["partial_json"]}}])
                        }
                        _ => {}
                    }
                }
                "message_delta" => {
                    self.usage["completion_tokens"] = event["usage"]["output_tokens"].clone();
                    finish = json!(match event["delta"]["stop_reason"].as_str() {
                        Some("tool_use") => "tool_calls",
                        Some("max_tokens") => "length",
                        _ => "stop",
                    });
                }
                "message_stop" => {
                    self.native = self.thinking.values().cloned().collect();
                }
                _ => {}
            },
            "generate-content" => {
                if let Some(parts) = event
                    .pointer("/candidates/0/content/parts")
                    .and_then(Value::as_array)
                {
                    for part in parts {
                        self.native.push(part.clone());
                        if part.get("functionCall").is_some() {
                            let call = &part["functionCall"];
                            delta["tool_calls"] = delta["tool_calls"]
                                .as_array()
                                .cloned()
                                .map(Value::Array)
                                .unwrap_or(json!([]));
                            delta["tool_calls"].as_array_mut().unwrap().push(json!({"index":self.calls,"id":call["id"].as_str().map(str::to_owned).unwrap_or_else(||format!("gemini_{}",self.calls)),"function":{"name":call["name"],"arguments":call["args"].to_string()}}));
                            self.calls += 1;
                        } else if let Some(text) = part["text"].as_str() {
                            let key = if part["thought"] == true {
                                "reasoning_content"
                            } else {
                                "content"
                            };
                            let joined = format!("{}{text}", delta[key].as_str().unwrap_or(""));
                            delta[key] = json!(joined);
                        }
                    }
                }
                if event.get("usageMetadata").is_some() {
                    let u = &event["usageMetadata"];
                    self.usage = json!({"prompt_tokens":u["promptTokenCount"],"completion_tokens":u["candidatesTokenCount"].as_u64().unwrap_or(0)+u["thoughtsTokenCount"].as_u64().unwrap_or(0),"completion_tokens_details":{"reasoning_tokens":u["thoughtsTokenCount"]},"prompt_tokens_details":{"cached_tokens":u["cachedContentTokenCount"]}});
                }
                if let Some(reason) = event
                    .pointer("/candidates/0/finishReason")
                    .and_then(Value::as_str)
                {
                    ensure!(
                        ["STOP", "MAX_TOKENS"].contains(&reason),
                        "provider blocked the response"
                    );
                    finish = json!(if reason == "MAX_TOKENS" {
                        "length"
                    } else if self.calls > 0 {
                        "tool_calls"
                    } else {
                        "stop"
                    });
                }
            }
            _ => anyhow::bail!("unsupported native transport"),
        }
        ensure!(
            self.native.len() <= 128 && serde_json::to_string(&self.native)?.len() <= 4_000_000,
            "provider replay data is too large"
        );
        Ok(json!({"choices":[{"delta":delta,"finish_reason":finish}],"usage":self.usage}))
    }
}
