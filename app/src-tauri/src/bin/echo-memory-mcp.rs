//! Local-only read-only MCP server. JSON-RPC travels exclusively over stdio.

use echo_memory_lib::library::ManagedLibrary;
use echo_memory_lib::state::default_library_root;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

fn main() {
    let library = match ManagedLibrary::open(default_library_root()) {
        Ok(library) => library,
        Err(error) => {
            eprintln!("echo-memory-mcp: {error}");
            return;
        }
    };
    for line in io::stdin().lock().lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let response = match method {
            "initialize" => {
                json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"echo-memory","version":"0.1.0"}}})
            }
            "tools/list" => json!({"jsonrpc":"2.0","id":id,"result":{"tools":tools()}}),
            "tools/call" => call_tool(&library, request.get("params").unwrap_or(&Value::Null), id),
            _ => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}})
            }
        };
        println!("{}", response);
        let _ = io::stdout().flush();
    }
}

fn tools() -> Vec<Value> {
    [
        ("search_records", "Search locally stored record titles and transcript snippets. Returns at most 10 results.", json!({"query":{"type":"string"},"project_id":{"type":"string"},"limit":{"type":"integer"}})),
        ("get_record", "Get record metadata and its latest transcript segments.", json!({"record_id":{"type":"string"}})),
        ("get_transcript", "Get a bounded range of transcript segments.", json!({"record_id":{"type":"string"},"start_ms":{"type":"integer"},"end_ms":{"type":"integer"}})),
        ("list_projects", "List local projects.", json!({})),
        ("get_project_context", "List recent records for a project.", json!({"project_id":{"type":"string"}})),
        ("list_action_items", "List local action items. Alpha currently returns no external writes.", json!({"project_id":{"type":"string"},"status":{"type":"string"}})),
        ("get_my_context", "Get the user's L3 profile summary, recently confirmed memory items, and minutes conclusions/action items the user explicitly confirmed (read-only personal context).", json!({})),
    ].into_iter().map(|(name, description, properties)| json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties}})).collect()
}

fn call_tool(library: &ManagedLibrary, params: &Value, id: Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").unwrap_or(&Value::Null);
    let repo = library.repository();
    match repo.mcp_enabled() {
        Ok(true) => {}
        Ok(false) => {
            return json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":"回声记忆的 AI 连接尚未启用。请先在应用左侧打开只读 MCP。"}],"isError":true}})
        }
        Err(error) => {
            return json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":error.to_string()}],"isError":true}})
        }
    }
    let result: Result<Value, String> = (|| match name {
        "search_records" => {
            let query = args.get("query").and_then(Value::as_str).unwrap_or("");
            let project_id = args.get("project_id").and_then(Value::as_str);
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(10)
                .min(10) as usize;
            repo.log_mcp_access(name, None, project_id)
                .map_err(|e| e.to_string())?;
            // 块4 导入确认门在 lib 层（mcp_search）；二进制只做 JSON-RPC 壳。
            repo.mcp_search(query, project_id, limit)
                .map(|items| json!(items))
                .map_err(|e| e.to_string())
        }
        "get_record" => {
            let record_id = required(args, "record_id")?;
            repo.log_mcp_access(name, Some(record_id), None)
                .map_err(|e| e.to_string())?;
            let (record, transcript) = repo.mcp_get_record(record_id).map_err(|e| e.to_string())?;
            Ok(json!({"record":record,"transcript":transcript}))
        }
        "get_transcript" => {
            let record_id = required(args, "record_id")?;
            let start = args.get("start_ms").and_then(Value::as_i64).unwrap_or(0);
            let end = args
                .get("end_ms")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX);
            repo.log_mcp_access(name, Some(record_id), None)
                .map_err(|e| e.to_string())?;
            repo.mcp_get_transcript(record_id, start, end)
                .map(|segments| json!(segments))
                .map_err(|e| e.to_string())
        }
        "list_projects" => {
            repo.log_mcp_access(name, None, None)
                .map_err(|e| e.to_string())?;
            repo.list_projects()
                .map(|items| json!(items))
                .map_err(|e| e.to_string())
        }
        "get_project_context" => {
            let project_id = required(args, "project_id")?;
            repo.log_mcp_access(name, None, Some(project_id))
                .map_err(|e| e.to_string())?;
            let (records, decisions, action_items) =
                repo.mcp_project_context(project_id).map_err(|e| e.to_string())?;
            Ok(
                json!({"records":records,"decisions":decisions,"action_items":action_items}),
            )
        }
        "list_action_items" => {
            let project_id = args.get("project_id").and_then(Value::as_str);
            let status = args.get("status").and_then(Value::as_str);
            repo.log_mcp_access(name, None, project_id)
                .map_err(|e| e.to_string())?;
            repo.list_action_items(project_id, status)
                .map(|items| json!(items))
                .map_err(|e| e.to_string())
        }
        "get_my_context" => {
            repo.log_mcp_access(name, None, None)
                .map_err(|e| e.to_string())?;
            let profile = repo
                .latest_profile_snapshot()
                .map_err(|e| e.to_string())?
                .map(|(snapshot, profile)| {
                    json!({
                        "summary": profile.summary,
                        "stances": profile.stances,
                        "preferences": profile.preferences,
                        "trajectory": profile.trajectory,
                        "watching": profile.watching,
                        "isStale": snapshot.is_stale,
                        "updatedAt": snapshot.updated_at,
                    })
                })
                .unwrap_or(Value::Null);
            let confirmed = repo.confirmed_memory_items(20).map_err(|e| e.to_string())?;
            // v1.1 块4：会议纪要已确认条目（同样只读 confirmed，未确认不可见）。
            let minutes_items = repo
                .confirmed_minutes_memory_entries(20)
                .map_err(|e| e.to_string())?;
            Ok(json!({"profile": profile, "confirmedItems": confirmed, "minutesItems": minutes_items}))
        }
        _ => Err("unknown tool".to_owned()),
    })();
    match result {
        Ok(value) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":value.to_string()}]}})
        }
        Err(message) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":message}],"isError":true}})
        }
    }
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}"))
}
