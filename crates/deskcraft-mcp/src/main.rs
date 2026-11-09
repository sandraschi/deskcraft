//! `deskcraft`: CLI and MCP stdio server in one binary.
//!
//! Every CLI verb builds the same tool-argument JSON an MCP client would send
//! and goes through `Command::from_tool` + `dispatch`, so CLI and MCP share one
//! code path. In `mcp` mode stdout carries only JSON-RPC; logs go to stderr.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use clap::{Parser, Subcommand};
use craft_biz::bus::Bus;
use craft_biz::mcp::{serve_stdio, tools_markdown, ToolHost};
use deskcraft_core::{dispatch, tools, Command, Confinement, Ctx, APP, VERSION};
use deskcraft_store::Desk;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const AFTER_HELP: &str = "Typical day:
  deskcraft ingest D:\\Mail\\Support            # .eml files or a maildir (cur/new)
  deskcraft list                               # open tickets, urgent first
  deskcraft show D-12
  deskcraft reply D-12 --macro geloest         # .eml draft in the outbox; nothing is sent
  deskcraft sent 41                            # after sending it from your mail client
  deskcraft sla                                # who is about to breach
  deskcraft escalate D-12 --project OPS        # becomes a taskcraft issue
See HELP.md for workflows and docs/TOOLS.md for every MCP tool.";

#[derive(Parser)]
#[command(name = "deskcraft", version, about = "Support tickets from e-mail: threading, SLA in business hours, reply drafts, macros, escalation. CLI + MCP.", after_help = AFTER_HELP)]
struct Cli {
    /// Database. Default: $DESKCRAFT_DB or %LOCALAPPDATA%\deskcraft\deskcraft.db
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    /// Outbox folder for reply drafts. Default: outbox next to the database.
    #[arg(long, global = true)]
    outbox: Option<PathBuf>,
    /// Print machine-readable JSON only.
    #[arg(long, global = true)]
    json: bool,
    /// Restrict file access to these directories (repeatable). Required for ingest in `mcp` mode.
    #[arg(long = "allow-path", global = true)]
    allow_path: Vec<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Counts, support address, outbox.
    Status,
    /// Show or set support address, sender name, signature.
    Config {
        #[arg(long)]
        address: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        signature: Option<String>,
    },
    /// Read .eml files or a maildir into tickets.
    Ingest {
        path: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// List tickets (default new, open, pending).
    List {
        /// Comma list of statuses.
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        priority: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// Show a ticket with messages, history and SLA.
    Show { ticket: String },
    /// Change status, priority, assignee, tags; add an internal note.
    Update {
        ticket: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        priority: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        unassign: bool,
        /// Comma list; replaces all tags.
        #[arg(long)]
        tags: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Write a reply draft (.eml) into the outbox.
    Reply {
        ticket: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long = "macro")]
        macro_name: Option<String>,
    },
    /// Mark a reply draft as sent.
    Sent {
        message: i64,
        #[arg(long)]
        at: Option<String>,
    },
    /// Reply macros.
    Macro {
        #[command(subcommand)]
        op: MacroOp,
    },
    /// SLA clocks of open tickets.
    Sla {
        #[arg(long)]
        as_of: Option<String>,
    },
    /// Send a ticket to taskcraft.
    Escalate {
        ticket: String,
        #[arg(long)]
        details: Option<String>,
        #[arg(long)]
        project: Option<String>,
    },
    /// Unhappy-customer signals; --emit sends them to gaincraft.
    Signals {
        #[arg(long)]
        emit: bool,
    },
    /// Fill an empty desk with labelled demo mails.
    DemoSeed,
    /// Print the MCP tool catalog (JSON, or markdown for docs/TOOLS.md).
    Tools {
        #[arg(long)]
        markdown: bool,
    },
    /// Run the MCP stdio server.
    Mcp,
}

#[derive(Subcommand)]
enum MacroOp {
    List,
    Set {
        name: String,
        body: String,
        #[arg(long)]
        status: Option<String>,
        /// Comma list.
        #[arg(long)]
        tags: Option<String>,
    },
}

fn default_db() -> PathBuf {
    if let Ok(p) = std::env::var("DESKCRAFT_DB") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("deskcraft").join("deskcraft.db")
}

fn default_outbox(db: &Path) -> PathBuf {
    db.parent()
        .map(|p| p.join("outbox"))
        .unwrap_or_else(|| PathBuf::from("outbox"))
}

fn obj(pairs: Vec<(&str, Option<Value>)>) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        if let Some(v) = v {
            m.insert(k.to_string(), v);
        }
    }
    Value::Object(m)
}
fn sv(s: &str) -> Option<Value> {
    Some(Value::String(s.to_string()))
}
fn os(s: Option<String>) -> Option<Value> {
    s.map(Value::String)
}

fn to_tool(cmd: Cmd) -> Result<(&'static str, Value), String> {
    Ok(match cmd {
        Cmd::Status => ("desk_status", json!({})),
        Cmd::Config {
            address,
            name,
            signature,
        } => (
            "desk_config",
            obj(vec![
                ("support_address", os(address)),
                ("support_name", os(name)),
                ("signature", os(signature)),
            ]),
        ),
        Cmd::Ingest { path, dry_run } => {
            ("desk_ingest", json!({ "path": path, "dry_run": dry_run }))
        }
        Cmd::List {
            status,
            priority,
            assignee,
            text,
            all,
        } => (
            "desk_ticket_list",
            obj(vec![
                ("status", os(status)),
                ("priority", os(priority)),
                ("assignee", os(assignee)),
                ("text", os(text)),
                ("include_closed", Some(json!(all))),
            ]),
        ),
        Cmd::Show { ticket } => ("desk_ticket_show", json!({ "ticket": ticket })),
        Cmd::Update {
            ticket,
            status,
            priority,
            assignee,
            unassign,
            tags,
            note,
        } => (
            "desk_ticket_update",
            obj(vec![
                ("ticket", sv(&ticket)),
                ("status", os(status)),
                ("priority", os(priority)),
                (
                    "assignee",
                    if unassign {
                        Some(Value::Null)
                    } else {
                        os(assignee)
                    },
                ),
                ("tags", os(tags)),
                ("note", os(note)),
            ]),
        ),
        Cmd::Reply {
            ticket,
            body,
            macro_name,
        } => (
            "desk_reply_draft",
            obj(vec![
                ("ticket", sv(&ticket)),
                ("body", os(body)),
                ("macro", os(macro_name)),
            ]),
        ),
        Cmd::Sent { message, at } => (
            "desk_reply_mark_sent",
            obj(vec![("message", Some(json!(message))), ("at", os(at))]),
        ),
        Cmd::Macro { op: MacroOp::List } => ("desk_macro_list", json!({})),
        Cmd::Macro {
            op:
                MacroOp::Set {
                    name,
                    body,
                    status,
                    tags,
                },
        } => (
            "desk_macro_set",
            obj(vec![
                ("name", sv(&name)),
                ("body", sv(&body)),
                ("set_status", os(status)),
                ("add_tags", os(tags)),
            ]),
        ),
        Cmd::Sla { as_of } => ("desk_sla_status", obj(vec![("as_of", os(as_of))])),
        Cmd::Escalate {
            ticket,
            details,
            project,
        } => (
            "desk_escalate",
            obj(vec![
                ("ticket", sv(&ticket)),
                ("details", os(details)),
                ("project", os(project)),
            ]),
        ),
        Cmd::Signals { emit } => ("desk_signals", json!({ "emit": emit })),
        Cmd::DemoSeed => ("desk_demo_seed", json!({})),
        Cmd::Tools { .. } | Cmd::Mcp => return Err("not a tool verb".into()),
    })
}

struct Host {
    ctx: Ctx,
}

impl ToolHost for Host {
    fn server_name(&self) -> &str {
        APP
    }
    fn server_version(&self) -> &str {
        VERSION
    }
    fn tools(&self) -> Vec<Value> {
        tools()
    }
    fn call(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        let cmd = Command::from_tool(name, args)?;
        dispatch(&mut self.ctx, cmd)
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "-".into(),
        Value::Array(a) => a.iter().map(scalar).collect::<Vec<_>>().join(","),
        other => other.to_string(),
    }
}

const COLUMN_ORDER: &[&str] = &[
    "key",
    "ticket",
    "priority",
    "status",
    "first_response",
    "first_response_due",
    "resolution",
    "resolution_due",
    "score",
    "subject",
    "requester_email",
    "requester",
    "assignee",
    "tags",
    "message_count",
    "opened_at",
    "name",
    "set_status",
    "add_tags",
    "hits",
];
const HIDDEN: &[&str] = &[
    "id",
    "requester_name",
    "first_response_at",
    "solved_at",
    "updated_at",
    "body",
    "sent_to_gaincraft",
];

fn columns(rows: &[Value]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for r in rows.iter().filter_map(Value::as_object) {
        for (k, v) in r {
            if !v.is_object() && !HIDDEN.contains(&k.as_str()) && !keys.contains(k) {
                keys.push(k.clone());
            }
        }
    }
    keys.sort_by_key(|k| {
        (
            COLUMN_ORDER
                .iter()
                .position(|c| c == k)
                .unwrap_or(usize::MAX),
            k.clone(),
        )
    });
    keys
}

fn table(rows: &[Value]) -> String {
    if rows.is_empty() {
        return "(none)\n".into();
    }
    let cols = columns(rows);
    let cut = |s: String| {
        if s.chars().count() > 60 {
            format!("{}...", s.chars().take(57).collect::<String>())
        } else {
            s
        }
    };
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            cols.iter()
                .map(|c| cut(r.get(c).map(scalar).unwrap_or_else(|| "-".into())))
                .collect()
        })
        .collect();
    let widths: Vec<usize> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| {
            cells
                .iter()
                .map(|r| r[i].chars().count())
                .chain([c.chars().count()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cs: &[String]| -> String {
        let p: Vec<String> = cs
            .iter()
            .zip(&widths)
            .map(|(c, w)| format!("{c}{}", " ".repeat(w.saturating_sub(c.chars().count()))))
            .collect();
        format!("{}\n", p.join("  ").trim_end())
    };
    let mut out = line(&cols);
    for r in &cells {
        out.push_str(&line(r));
    }
    out
}

/// Human rendering: a ticket as a conversation, lists as tables, the rest as JSON.
fn render(v: &Value) -> String {
    let Some(obj) = v.as_object() else {
        return scalar(v);
    };
    if let Some(msgs) = obj.get("messages").and_then(Value::as_array) {
        let mut out = format!(
            "{}  {}\n{} | {} | {} | assignee {} | tags {}\nfirst response: {}   resolution: {}\n",
            scalar(&obj["key"]),
            scalar(&obj["subject"]),
            scalar(&obj["requester_email"]),
            scalar(&obj["status"]),
            scalar(&obj["priority"]),
            scalar(&obj["assignee"]),
            scalar(&obj["tags"]),
            scalar(&obj["sla"]["first_response"]["state"]),
            scalar(&obj["sla"]["resolution"]["state"])
        );
        for m in msgs {
            let kind = match (m["direction"].as_str(), m["draft"].as_bool()) {
                (Some("note"), _) => "NOTE",
                (Some("out"), Some(true)) => "DRAFT",
                (Some("out"), _) => "OUT",
                _ => "IN",
            };
            out.push_str(&format!(
                "\n--- #{} {kind} {} {}\n{}\n",
                scalar(&m["id"]),
                scalar(&m["date"]),
                scalar(&m["from_email"]),
                scalar(&m["body"])
            ));
        }
        return out;
    }
    const LISTS: [&str; 3] = ["tickets", "rows", "macros"];
    let Some(key) = LISTS
        .iter()
        .find(|k| obj.get(**k).is_some_and(Value::is_array))
    else {
        return serde_json::to_string_pretty(v).unwrap_or_else(|e| e.to_string());
    };
    let mut out = String::new();
    for (k, val) in obj {
        if !val.is_array() && !val.is_object() {
            out.push_str(&format!("{k}: {}\n", scalar(val)));
        }
    }
    out.push_str(&table(
        obj.get(*key)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    ));
    out
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Cmd::Tools { markdown } = cli.cmd {
        if markdown {
            print!("{}", tools_markdown(APP, &tools()));
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&tools()).unwrap_or_default()
            );
        }
        return ExitCode::SUCCESS;
    }
    let db_path = cli.db.clone().unwrap_or_else(default_db);
    let desk = match Desk::open(&db_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("deskcraft: cannot open {}: {e}", db_path.display());
            return ExitCode::FAILURE;
        }
    };
    let is_mcp = matches!(cli.cmd, Cmd::Mcp);
    let confinement = if is_mcp || !cli.allow_path.is_empty() {
        match Confinement::roots(&cli.allow_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("deskcraft: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        Confinement(None)
    };
    let actor = if is_mcp {
        "agent".to_string()
    } else {
        std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "cli".into())
    };
    let outbox = cli
        .outbox
        .clone()
        .unwrap_or_else(|| default_outbox(&db_path));
    let ctx = Ctx {
        desk,
        confinement,
        bus: Bus::new(Bus::default_root()),
        outbox,
        actor,
    };
    if is_mcp {
        eprintln!("[deskcraft] db {}", db_path.display());
        let mut host = Host { ctx };
        return match serve_stdio(&mut host) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("deskcraft mcp: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let json_out = cli.json;
    let mut ctx = ctx;
    let result = to_tool(cli.cmd)
        .and_then(|(name, args)| Command::from_tool(name, &args))
        .and_then(|cmd| dispatch(&mut ctx, cmd));
    match result {
        Ok(v) => {
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            } else {
                print!("{}", render(&v));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json_out {
                println!("{}", json!({ "error": e }));
            } else {
                eprintln!("deskcraft: {e}");
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_biz::mcp::{docs_in_sync, handle_line};

    fn host(tag: &str) -> Host {
        let dir = std::env::temp_dir().join(format!("deskcraft-mcp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Host {
            ctx: Ctx {
                desk: Desk::open_in_memory().unwrap(),
                confinement: Confinement(Some(Vec::new())),
                bus: Bus::new(dir.join("bus")),
                outbox: dir.join("outbox"),
                actor: "agent".into(),
            },
        }
    }

    fn text(v: &Value) -> String {
        v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    #[test]
    fn mcp_probe_initialize_list_call() {
        let mut h = host("probe");
        let init = handle_line(
            &mut h,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        )
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "deskcraft");
        let list =
            handle_line(&mut h, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        assert_eq!(list["result"]["tools"].as_array().map(Vec::len), Some(14));
        let seed = handle_line(&mut h, r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"desk_demo_seed","arguments":{}}}"#).unwrap();
        assert_eq!(seed["result"]["isError"], false, "{seed}");
        let show = handle_line(&mut h, r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"desk.ticket_show","arguments":{"ticket":"D-1"}}}"#).unwrap();
        assert!(text(&show).contains("Papierstau"));
        let ingest = handle_line(&mut h, r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"desk_ingest","arguments":{"path":"C:/Windows"}}}"#).unwrap();
        assert_eq!(
            ingest["result"]["isError"], true,
            "no file access without --allow-path"
        );
        let upd = handle_line(&mut h, r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"desk_ticket_update","arguments":{"ticket":"D-2","priority":"urgent"}}}"#).unwrap();
        assert_eq!(upd["result"]["isError"], false);
        let hist: Value = serde_json::from_str(&text(&handle_line(&mut h, r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"desk_ticket_show","arguments":{"ticket":"D-2"}}}"#).unwrap())).unwrap();
        assert!(hist["history"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["actor"] == "agent" && r["field"] == "priority"));
        if let Some(dir) = h.ctx.outbox.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn cli_verbs_map_to_known_tools() {
        let verbs = vec![
            Cmd::Status,
            Cmd::List {
                status: None,
                priority: None,
                assignee: None,
                text: None,
                all: false,
            },
            Cmd::Reply {
                ticket: "D-1".into(),
                body: None,
                macro_name: Some("geloest".into()),
            },
            Cmd::Sent {
                message: 1,
                at: None,
            },
            Cmd::Macro { op: MacroOp::List },
            Cmd::Sla { as_of: None },
            Cmd::Escalate {
                ticket: "D-1".into(),
                details: None,
                project: None,
            },
            Cmd::Signals { emit: false },
            Cmd::DemoSeed,
        ];
        let names: Vec<String> = tools()
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect();
        for v in verbs {
            let (name, _) = to_tool(v).unwrap();
            assert!(names.iter().any(|n| n == name), "{name}");
        }
    }

    #[test]
    fn tools_doc_is_current() {
        let committed = include_str!("../../../docs/TOOLS.md");
        assert!(
            docs_in_sync(committed, &tools_markdown(APP, &tools())),
            "docs/TOOLS.md is stale: run `deskcraft tools --markdown > docs/TOOLS.md`"
        );
    }

    #[test]
    fn render_ticket_conversation() {
        let v = json!({ "key": "D-1", "subject": "S", "requester_email": "a@b", "status": "open", "priority": "normal", "assignee": null, "tags": [], "sla": { "first_response": { "state": "met" }, "resolution": { "state": "ok" } }, "messages": [ { "id": 1, "direction": "in", "draft": false, "date": "d", "from_email": "a@b", "body": "Hallo" } ] });
        let s = render(&v);
        assert!(s.contains("--- #1 IN") && s.contains("Hallo"));
    }
}
