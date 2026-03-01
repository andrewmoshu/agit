// MCP Server — JSON-RPC over stdio
//
// Exposes agit operations as MCP tools:
//   agit_read(file_path, depth?)         → knowledge + recent log entries
//   agit_write(file_path, entry)         → append to log
//   agit_status()                        → staleness report
//   agit_compact(file_path)              → returns compaction prompt for the agent to process
//   agit_compact_finish(file_path, body) → writes the agent's compaction result
//
// Architecture:
//   - Reads JSON-RPC requests from stdin (line-delimited)
//   - Dispatches to the same core:: functions the CLI uses
//   - Writes JSON-RPC responses to stdout
//   - Capabilities announced via MCP's initialize handshake
//
// Compaction flow:
//   The agent IS the LLM. agit doesn't call any external API.
//   1. Agent calls agit_compact(file) → gets the prompt + context
//   2. Agent processes it with its own intelligence
//   3. Agent calls agit_compact_finish(file, new_body) → agit writes the result

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::io::{self, BufRead, Write};

use crate::config::Config;
use crate::core::compact::{finish_compaction, prepare_compaction};
use crate::core::knowledge::KnowledgeFile;
use crate::core::log::{Confidence, EntryType, LogEntry, LogFile};
use crate::core::shadow::{Shadow, TargetScope};
use crate::core::staleness::check_all_staleness;
use crate::cli::seed::gather_seed_data;

// --- JSON-RPC types ---

#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

// --- MCP protocol types ---

#[derive(Serialize)]
struct McpToolInfo {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

// --- Server ---

pub fn serve() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    eprintln!("agit MCP server started. Listening on stdin...");

    for line in stdin.lock().lines() {
        let line = line.context("reading stdin")?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: None,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("parse error: {}", e),
                    }),
                };
                writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
                stdout.flush()?;
                continue;
            }
        };

        let response = handle_request(&request);
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }

    Ok(())
}

fn handle_request(req: &JsonRpcRequest) -> JsonRpcResponse {
    let result = match req.method.as_str() {
        "initialize" => handle_initialize(),
        "tools/list" => handle_tools_list(),
        "tools/call" => handle_tool_call(&req.params),
        _ => Err(anyhow::anyhow!("unknown method: {}", req.method)),
    };

    match result {
        Ok(value) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: req.id.clone(),
            result: Some(value),
            error: None,
        },
        Err(e) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: req.id.clone(),
            result: None,
            error: Some(JsonRpcError {
                code: -32000,
                message: format!("{:#}", e),
            }),
        },
    }
}

fn handle_initialize() -> Result<Value> {
    Ok(serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "agit",
            "version": env!("CARGO_PKG_VERSION")
        }
    }))
}

fn handle_tools_list() -> Result<Value> {
    let tools = vec![
        McpToolInfo {
            name: "agit_read".into(),
            description: "Read knowledge for a file, directory, or the project. For files: returns hierarchical knowledge (root → directory → file) and recent log entries. For directories (trailing '/'): returns directory-level knowledge and log. For root ('/'): returns project-level knowledge and log. Call this before modifying code to understand why it is the way it is.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Target path. File: 'src/auth/login.ts'. Directory: 'src/auth/'. Project root: '/'"
                    },
                    "depth": {
                        "type": "string",
                        "enum": ["shallow", "deep", "knowledge_only"],
                        "description": "Read depth. shallow (default): knowledge + last 5 log entries. deep: full history. knowledge_only: no log entries."
                    }
                },
                "required": ["file_path"]
            }),
        },
        McpToolInfo {
            name: "agit_write".into(),
            description: "Record knowledge about a file, directory, or the project. Use file path for file-level insights, path with trailing '/' for directory-level patterns, or '/' for project-wide conventions.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Target path. File: 'src/auth/login.ts'. Directory: 'src/auth/'. Project root: '/'"
                    },
                    "agent": {
                        "type": "string",
                        "description": "Agent identifier (e.g. 'claude-code', 'codex')"
                    },
                    "type": {
                        "type": "string",
                        "enum": ["insight", "decision", "failure", "constraint", "relationship"],
                        "description": "Category of knowledge"
                    },
                    "content": {
                        "type": "string",
                        "description": "The insight in natural language"
                    },
                    "confidence": {
                        "type": "string",
                        "enum": ["observed", "inferred"],
                        "description": "observed = fact, inferred = theory. Default: observed"
                    },
                    "anchors": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Code element names this relates to (function names, class names)"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Freeform tags for filtering"
                    }
                },
                "required": ["file_path", "agent", "type", "content"]
            }),
        },
        McpToolInfo {
            name: "agit_status".into(),
            description: "Show compaction status across the project. Reports which files, directories, and root have accumulated log entries exceeding the compaction threshold.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        McpToolInfo {
            name: "agit_compact".into(),
            description: "Get the compaction prompt for a file, directory, or root. Returns context formatted as a prompt. YOU (the agent) process this prompt and call agit_compact_finish with the result.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Target path. File: 'src/auth/login.ts'. Directory: 'src/auth/'. Project root: '/'"
                    }
                },
                "required": ["file_path"]
            }),
        },
        McpToolInfo {
            name: "agit_compact_finish".into(),
            description: "Write the result of compaction. Call this after processing the prompt from agit_compact. Writes the new knowledge file, archives the old one, and clears the log.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Target path (same as used in agit_compact). File: 'src/auth/login.ts'. Directory: 'src/auth/'. Project root: '/'"
                    },
                    "new_body": {
                        "type": "string",
                        "description": "The new knowledge file body (markdown, no frontmatter)"
                    }
                },
                "required": ["file_path", "new_body"]
            }),
        },
        McpToolInfo {
            name: "agit_seed".into(),
            description: "Gather raw data from a source for seeding knowledge. Returns data for YOU (the agent) to analyze. For 'git' and 'comments': extract insights, call agit_write() for each. For 'scan': get a manifest of all files needing knowledge, then read files and bootstrap them.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "enum": ["scan", "git", "comments"],
                        "description": "Where to gather seed data from. 'scan': project file manifest showing what needs knowledge (START HERE). 'git': commit history (reverts, fixes, churn). 'comments': TODO/HACK/FIXME/NOTE in code."
                    }
                },
                "required": ["source"]
            }),
        },
    ];

    Ok(serde_json::json!({ "tools": tools }))
}

fn handle_tool_call(params: &Value) -> Result<Value> {
    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .context("missing tool name")?;

    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    let cwd = env::current_dir()?;
    let project_root = Config::find_project_root(&cwd)
        .context("not in an agit project (run `agit init` first)")?;
    let config = Config::load(&project_root)?;
    let shadow = Shadow::new(project_root);

    match tool_name {
        "agit_read" => tool_read(&args, &shadow, &config),
        "agit_write" => tool_write(&args, &shadow),
        "agit_status" => tool_status(&shadow, &config),
        "agit_compact" => tool_compact(&args, &shadow),
        "agit_compact_finish" => tool_compact_finish(&args, &shadow),
        "agit_seed" => tool_seed(&args, &shadow),
        _ => anyhow::bail!("unknown tool: {}", tool_name),
    }
}

fn tool_read(args: &Value, shadow: &Shadow, config: &Config) -> Result<Value> {
    let target = args
        .get("file_path")
        .and_then(|v| v.as_str())
        .context("missing file_path")?;

    let depth = args
        .get("depth")
        .and_then(|v| v.as_str())
        .unwrap_or("shallow");

    let scope = TargetScope::parse(target);
    let mut output = String::new();

    match &scope {
        TargetScope::File(_) => {
            let hierarchy = shadow.knowledge_hierarchy(target);
            for path in &hierarchy {
                if path.exists() {
                    let kf = KnowledgeFile::load(path)?;
                    if !output.is_empty() {
                        output.push_str("\n---\n\n");
                    }
                    output.push_str(&kf.body);
                }
            }
        }
        TargetScope::Dir(d) => {
            let path = shadow.dir_knowledge_path(d);
            if path.exists() {
                let kf = KnowledgeFile::load(&path)?;
                output.push_str(&kf.body);
            }
        }
        TargetScope::Root => {
            let path = shadow.root_knowledge_path();
            if path.exists() {
                let kf = KnowledgeFile::load(&path)?;
                output.push_str(&kf.body);
            }
        }
    }

    if depth != "knowledge_only" {
        let log_path = shadow.resolve_log_path(target);
        let log = LogFile::new(log_path);
        if log.exists() {
            let entries = if depth == "deep" {
                log.read_all()?
            } else {
                log.read_tail(config.read.shallow_log_entries)?
            };

            if !entries.is_empty() {
                output.push_str("\n---\n\n## Recent Log\n\n");
                for entry in &entries {
                    let confidence_marker = if entry.confidence == Confidence::Inferred {
                        " [inferred]"
                    } else {
                        ""
                    };
                    let anchors = if entry.anchors.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", entry.anchors.join(", "))
                    };
                    output.push_str(&format!(
                        "- **[{}]** {}{}{}\n",
                        entry.entry_type, entry.content, anchors, confidence_marker
                    ));
                }
            }
        }
    }

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": if output.trim().is_empty() {
                format!("No knowledge found for {}.", target)
            } else {
                output
            }
        }]
    }))
}

fn tool_write(args: &Value, shadow: &Shadow) -> Result<Value> {
    let file = args.get("file_path").and_then(|v| v.as_str()).context("missing file_path")?;
    let agent = args.get("agent").and_then(|v| v.as_str()).context("missing agent")?;
    let entry_type: EntryType = args.get("type").and_then(|v| v.as_str()).context("missing type")?.parse()?;
    let content = args.get("content").and_then(|v| v.as_str()).context("missing content")?;

    let confidence: Confidence = args
        .get("confidence")
        .and_then(|v| v.as_str())
        .unwrap_or("observed")
        .parse()?;

    let anchors: Vec<String> = args
        .get("anchors")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let entry = LogEntry::new(agent.into(), entry_type, content.into(), confidence, anchors, tags);
    let log = LogFile::new(shadow.resolve_log_path(file));
    log.append(&entry)?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": format!("Recorded {} for {}.", entry.entry_type, file)
        }]
    }))
}

fn tool_status(shadow: &Shadow, config: &Config) -> Result<Value> {
    let reports = check_all_staleness(shadow, config)?;
    let mut output = String::new();

    let tracked = shadow.tracked_files()?;
    output.push_str(&format!("agit status: {} tracked files\n\n", tracked.len()));

    for report in &reports {
        output.push_str(&format!("{}", report));
    }

    if reports.iter().any(|r| r.is_stale()) {
        output.push_str("\nRun agit_compact on stale files to synthesize knowledge.\n");
    }

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": output
        }]
    }))
}

fn tool_compact(args: &Value, shadow: &Shadow) -> Result<Value> {
    let file = args.get("file_path").and_then(|v| v.as_str()).context("missing file_path")?;
    let ctx = prepare_compaction(file, shadow)?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": ctx.prompt
        }],
        "_agit_meta": {
            "action": "compact",
            "file": file,
            "instruction": "Process the above prompt and call agit_compact_finish with the resulting markdown body."
        }
    }))
}

fn tool_compact_finish(args: &Value, shadow: &Shadow) -> Result<Value> {
    let file = args.get("file_path").and_then(|v| v.as_str()).context("missing file_path")?;
    let new_body = args.get("new_body").and_then(|v| v.as_str()).context("missing new_body")?;

    finish_compaction(file, new_body, shadow)?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": format!("Compaction complete for {}. Knowledge updated, log cleared, old version archived.", file)
        }]
    }))
}

fn tool_seed(args: &Value, shadow: &Shadow) -> Result<Value> {
    let source = args
        .get("source")
        .and_then(|v| v.as_str())
        .context("missing source")?;

    let data = gather_seed_data(shadow, source)?;

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": data
        }],
        "_agit_meta": {
            "action": "seed",
            "source": source,
            "instruction": "Analyze the above data and call agit_write() for each meaningful insight you extract."
        }
    }))
}
