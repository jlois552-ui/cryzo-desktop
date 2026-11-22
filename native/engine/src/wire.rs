//! Native wire adapters. The agent's canonical conversation remains provider-independent.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn prepare(protocol: &str, mut request: Value) -> Result<(String, Value)> {
    let messages = request["messages"]
        .as_array()
        .context("messages missing")?
        .clone();
    let messages = &messages;
    if protocol == "chat-completions" {
        request["messages"] = json!(chat_messages(messages));
        request["stream"] = json!(true);
        return Ok(("chat/completions".into(), request));
    }
    if protocol == "responses" {
        let mut input = Vec::new();
        for message in messages {
            if message["role"] == "tool" {
                let images = images(message);
                let output = if images.is_empty() {
                    message["content"].clone()
                } else {
                    let mut parts = vec![json!({"type":"input_text","text":message["content"]})];
                    parts.extend(images.iter().map(|(mime, data)| json!({"type":"input_image","image_url":format!("data:{mime};base64,{data}")})));
                    json!(parts)
                };
                input.push(json!({"type":"function_call_output","call_id":message["tool_call_id"],"output":output}));
                continue;
            }
            if let Some(native) = message["cryzo_native"]["responses"].as_array() {
                input.extend(native.iter().cloned());
            }
            let content = message["content"].as_str().unwrap_or("");
            let images = images(message);
            if !images.is_empty() {
                let mut parts = vec![json!({"type":"input_text","text":content})];
                parts.extend(images.iter().map(|(mime, data)| json!({"type":"input_image","image_url":format!("data:{mime};base64,{data}")})));
                input.push(json!({"role":message["role"],"content":parts}));
            } else if !content.is_empty() {
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
                let images = images(message);
                let content = if images.is_empty() {
                    message["content"].clone()
                } else {
                    let mut parts = vec![json!({"type":"text","text":message["content"]})];
                    parts.extend(images.iter().map(|(mime, data)| json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}})));
                    json!(parts)
                };
                append_turn(
                    &mut conversation,
                    "user",
                    "content",
                    vec![
                        json!({"type":"tool_result","tool_use_id":message["tool_call_id"],"content":content}),
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
            content.extend(images(message).iter().map(|(mime, data)| json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}})));
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
                let mut response = json!({"name":name,"response":{"output":message["content"]}});
                let images = images(message);
                if !images.is_empty() {
                    response["parts"] = json!(
                        images
                            .iter()
                            .map(|(mime, data)| json!({"inlineData":{"mimeType":mime,"data":data}}))
                            .collect::<Vec<_>>()
                    );
                }
                append_turn(
                    &mut contents,
                    "user",
                    "parts",
                    vec![json!({ "functionResponse": response })],
                );
                continue;
            }
            let mut parts = Vec::new();
            if let Some(text) = message["content"].as_str().filter(|t| !t.is_empty()) {
                parts.push(json!({"text":text}));
            }
            parts.extend(
                images(message)
                    .iter()
                    .map(|(mime, data)| json!({"inlineData":{"mimeType":mime,"data":data}})),
            );
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

/// Images a tool returned (`cryzo_images`, data URLs) as media type and base64 data.
fn images(message: &Value) -> Vec<(String, String)> {
    message["cryzo_images"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|url| url.strip_prefix("data:")?.split_once(";base64,"))
        .filter(|(mime, _)| mime.starts_with("image/"))
        .map(|(mime, data)| (mime.to_owned(), data.to_owned()))
        .collect()
}

/// Chat Completions tool messages hold text only, so images a run of tool results returned follow
/// it as one user message.
fn chat_messages(messages: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut pending = Vec::new();
    let flush = |out: &mut Vec<Value>, pending: &mut Vec<Value>| {
        if pending.is_empty() {
            return;
        }
        let mut content =
            vec![json!({"type":"text","text":"Images returned by the tool calls above."})];
        content.append(pending);
        out.push(json!({"role":"user","content":content}));
    };
    for message in messages {
        let mut message = message.clone();
        if message["role"] == "tool" {
            pending.extend(images(&message).into_iter().map(|(mime, data)| json!({"type":"image_url","image_url":{"url":format!("data:{mime};base64,{data}")}})));
            if let Some(fields) = message.as_object_mut() {
                fields.remove("cryzo_images");
            }
            out.push(message);
            continue;
        }
        flush(&mut out, &mut pending);
        let attached = images(&message);
        if let Some(fields) = message.as_object_mut() {
            fields.remove("cryzo_images");
        }
        if !attached.is_empty() {
            let mut content = vec![json!({"type":"text","text":message["content"]})];
            content.extend(attached.iter().map(|(mime, data)| json!({"type":"image_url","image_url":{"url":format!("data:{mime};base64,{data}")}})));
            message["content"] = json!(content);
        }
        out.push(message);
    }
    flush(&mut out, &mut pending);
    out
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
            "{}",
            provider_error(&event)
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
                "response.failed" => anyhow::bail!("{}", provider_error(&event)),
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

/// What the provider said went wrong, such as a context that is too long, rather than a bare
/// "failed": the message is in `error`, or in `response.error` for the Responses API.
pub(crate) fn provider_error(event: &Value) -> String {
    let message = [
        event.pointer("/error/message"),
        event.pointer("/response/error/message"),
        event.pointer("/message"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)
    .or_else(|| event.get("error").and_then(Value::as_str));
    match message {
        Some(message) => format!("the model provider reported an error: {message}"),
        None => "provider reported a generation error".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::prepare;
    use serde_json::{Value, json};

    /// A conversation where the agent took a screenshot: the tool result carries one image.
    fn request() -> Value {
        json!({"model":"m","messages":[
            {"role":"system","content":"be brief"},
            {"role":"user","content":"check the page"},
            {"role":"assistant","content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"page_screenshot","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"c1","content":"Screenshot of http://localhost:5173/","cryzo_images":["data:image/jpeg;base64,QUJD"]},
            {"role":"assistant","content":"Looks fine."},
        ]})
    }

    #[test]
    fn chat_completions_follows_tool_results_with_their_images() {
        let (_, body) = prepare("chat-completions", request()).unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[3]["role"], "tool");
        assert!(messages[3].get("cryzo_images").is_none());
        assert_eq!(messages[4]["role"], "user");
        assert_eq!(
            messages[4]["content"][1],
            json!({"type":"image_url","image_url":{"url":"data:image/jpeg;base64,QUJD"}})
        );
        assert_eq!(messages[5]["content"], "Looks fine.");
    }

    #[test]
    fn responses_put_the_image_in_the_function_output() {
        let (_, body) = prepare("responses", request()).unwrap();
        let output = body["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "function_call_output")
            .unwrap();
        assert_eq!(
            output["output"],
            json!([
                {"type":"input_text","text":"Screenshot of http://localhost:5173/"},
                {"type":"input_image","image_url":"data:image/jpeg;base64,QUJD"},
            ])
        );
    }

    #[test]
    fn messages_put_the_image_in_the_tool_result() {
        let (_, body) = prepare("messages", request()).unwrap();
        let result = &body["messages"][2]["content"][0];
        assert_eq!(result["type"], "tool_result");
        assert_eq!(
            result["content"][1],
            json!({"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"QUJD"}})
        );
    }

    #[test]
    fn generate_content_puts_the_image_in_the_function_response() {
        let (_, body) = prepare("generate-content", request()).unwrap();
        let response = &body["contents"][2]["parts"][0]["functionResponse"];
        assert_eq!(response["name"], "page_screenshot");
        assert_eq!(
            response["parts"],
            json!([{"inlineData":{"mimeType":"image/jpeg","data":"QUJD"}}])
        );
    }

    #[test]
    fn a_user_message_carries_its_images_in_every_format() {
        let request = json!({"model":"m","messages":[
            {"role":"user","content":"what is this?","cryzo_images":["data:image/png;base64,QUJD"]},
        ]});
        let (_, chat) = prepare("chat-completions", request.clone()).unwrap();
        assert_eq!(
            chat["messages"][0]["content"],
            json!([{"type":"text","text":"what is this?"},{"type":"image_url","image_url":{"url":"data:image/png;base64,QUJD"}}])
        );
        assert!(chat["messages"][0].get("cryzo_images").is_none());
        let (_, responses) = prepare("responses", request.clone()).unwrap();
        assert_eq!(
            responses["input"][0]["content"][1],
            json!({"type":"input_image","image_url":"data:image/png;base64,QUJD"})
        );
        let (_, messages) = prepare("messages", request.clone()).unwrap();
        assert_eq!(
            messages["messages"][0]["content"][1],
            json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}})
        );
        let (_, gemini) = prepare("generate-content", request).unwrap();
        assert_eq!(
            gemini["contents"][0]["parts"][1],
            json!({"inlineData":{"mimeType":"image/png","data":"QUJD"}})
        );
    }

    #[test]
    fn tool_results_without_images_stay_text() {
        let mut plain = request();
        plain["messages"][3]
            .as_object_mut()
            .unwrap()
            .remove("cryzo_images");
        let (_, body) = prepare("chat-completions", plain.clone()).unwrap();
        assert_eq!(body["messages"].as_array().unwrap().len(), 5);
        let (_, body) = prepare("messages", plain).unwrap();
        assert_eq!(
            body["messages"][2]["content"][0]["content"],
            "Screenshot of http://localhost:5173/"
        );
    }
}
