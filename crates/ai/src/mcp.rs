use crate::tools::*;
use serde_json::{Value, json};
#[derive(Default)]
pub struct McpSession {
    initialized: bool,
}
pub enum McpStep {
    Reply(Value),
    Call {
        id: Value,
        call: TerminalCall,
        arguments: Value,
    },
    Rejected {
        id: Value,
        tool: String,
        arguments: Value,
        error: String,
    },
    Ignore,
}
fn error(id: Value, code: i64, message: &str) -> McpStep {
    McpStep::Reply(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}))
}
pub fn tool_result(id: Value, result: Result<Value, String>) -> Value {
    match result {
        Ok(value) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":value.to_string()}]}})
        }
        Err(message) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"isError":true,"content":[{"type":"text","text":crate::redact::redact(&message)}]}})
        }
    }
}
impl McpSession {
    pub fn handle(&mut self, line: &str) -> McpStep {
        if line.len() > MAX_MCP_LINE {
            return error(Value::Null, -32600, "Request exceeds limit");
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return error(Value::Null, -32700, "Invalid JSON");
        };
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        if !value.is_object()
            || value.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || !matches!(id, Value::Null | Value::String(_) | Value::Number(_))
        {
            return error(Value::Null, -32600, "Invalid request");
        }
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            return error(id, -32600, "Missing method");
        };
        if !value.as_object().expect("object").contains_key("id") {
            return McpStep::Ignore;
        }
        match method {
            "initialize" => {
                self.initialized = true;
                McpStep::Reply(
                    json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2024-11-05","serverInfo":{"name":"nocterm","version":"1"},"capabilities":{"tools":{}}}}),
                )
            }
            "ping" => McpStep::Reply(json!({"jsonrpc":"2.0","id":id,"result":{}})),
            _ if !self.initialized => error(id, -32002, "Initialize first"),
            "tools/list" => {
                let tools: Vec<_> = catalog::TOOLS.iter().map(|spec| {
                    json!({"name":spec.name,"description":spec.description,"inputSchema":(spec.schema)()})
                }).collect();
                McpStep::Reply(json!({"jsonrpc":"2.0","id":id,"result":{"tools":tools}}))
            }
            "tools/call" => call(id, &value["params"]),
            _ => error(id, -32601, "Unknown method"),
        }
    }
}

fn call(id: Value, params: &Value) -> McpStep {
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let tool = params["name"].as_str().unwrap_or_default().to_owned();
    let call = catalog::get(&tool).and_then(|spec| (spec.parse)(args.clone()));
    match call {
        Some(call) => match call.validate() {
            Ok(()) => McpStep::Call {
                id,
                call,
                arguments: args,
            },
            Err(error) => McpStep::Rejected {
                id,
                tool,
                arguments: args,
                error,
            },
        },
        None => McpStep::Rejected {
            id,
            tool,
            arguments: args,
            error: "Unknown tool or invalid arguments".into(),
        },
    }
}
