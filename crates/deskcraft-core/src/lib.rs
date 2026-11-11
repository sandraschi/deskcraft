//! deskcraft-core: the command bus shared by CLI, MCP and the desktop app.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod mail;
pub mod sla;

use chrono::{Duration, Local, NaiveDateTime};
use craft_biz::at::parse_date;
use craft_biz::bus::Bus;
pub use craft_biz::confine::Confinement;
use craft_biz::mcp::tool;
use deskcraft_store::{Desk, DeskError, NewMessage, Priority, Status, Ticket};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const APP: &str = "deskcraft";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_FILES: usize = 5000;
const MAX_MAIL_BYTES: u64 = 25 * 1024 * 1024;
/// Subject fallback only joins tickets opened within this many days.
const SUBJECT_WINDOW_DAYS: i64 = 30;

/// Deterministic negative-signal words (DE/EN). Strong words count double.
const NEGATIVE: &[&str] = &[
    "unzufrieden",
    "enttäusch",
    "frechheit",
    "katastroph",
    "unverschämt",
    "ärgerlich",
    "beschwerde",
    "schon wieder",
    "inakzeptabel",
    "disappointed",
    "unacceptable",
    "terrible",
    "angry",
    "complaint",
    "frustrat",
    "again and again",
    "worst",
];
const STRONG: &[&str] = &[
    "kündig",
    "storn",
    "anwalt",
    "rücktritt",
    "geld zurück",
    "cancel",
    "refund",
    "lawyer",
    "terminate",
];

pub struct Ctx {
    pub desk: Desk,
    pub confinement: Confinement,
    pub bus: Bus,
    /// Where reply drafts (.eml) are written.
    pub outbox: PathBuf,
    pub actor: String,
}

#[derive(Debug, Clone)]
pub enum Command {
    Status,
    Config {
        support_address: Option<String>,
        support_name: Option<String>,
        signature: Option<String>,
    },
    Ingest {
        path: String,
        dry_run: bool,
    },
    List {
        status: Vec<Status>,
        priority: Option<Priority>,
        assignee: Option<String>,
        text: Option<String>,
        all: bool,
    },
    Show {
        ticket: String,
    },
    Update {
        ticket: String,
        status: Option<Status>,
        priority: Option<Priority>,
        assignee: Option<Option<String>>,
        tags: Option<Vec<String>>,
        note: Option<String>,
    },
    ReplyDraft {
        ticket: String,
        body: Option<String>,
        macro_name: Option<String>,
    },
    ReplyMarkSent {
        message: i64,
        at: Option<NaiveDateTime>,
    },
    MacroSet {
        name: String,
        body: String,
        set_status: Option<Status>,
        add_tags: Vec<String>,
    },
    MacroList,
    Sla {
        as_of: Option<NaiveDateTime>,
    },
    Escalate {
        ticket: String,
        details: String,
        project: Option<String>,
    },
    Signals {
        emit: bool,
    },
    DemoSeed,
}

// ---------- argument helpers ----------

fn s<'a>(a: &'a Value, k: &str) -> Result<&'a str, String> {
    a.get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string argument {k:?}"))
}
fn opt_s(a: &Value, k: &str) -> Option<String> {
    a.get(k)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|v| !v.trim().is_empty())
}
fn e(err: DeskError) -> String {
    err.to_string()
}
fn flag(a: &Value, k: &str) -> bool {
    a.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn now() -> NaiveDateTime {
    Local::now().naive_local()
}
fn datetime(v: &str) -> Result<NaiveDateTime, String> {
    let t = v.trim();
    for f in [
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%d.%m.%Y %H:%M",
    ] {
        if let Ok(x) = NaiveDateTime::parse_from_str(t, f) {
            return Ok(x);
        }
    }
    parse_date(t)
        .map_err(|x| x.to_string())?
        .and_hms_opt(23, 59, 0)
        .ok_or_else(|| "bad date".into())
}
fn opt_dt(a: &Value, k: &str) -> Result<Option<NaiveDateTime>, String> {
    opt_s(a, k).map(|v| datetime(&v)).transpose()
}
fn str_list(a: &Value, k: &str) -> Result<Option<Vec<String>>, String> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(v)) => v
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("{k} must be strings"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(Value::String(t)) => Ok(Some(
            t.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect(),
        )),
        _ => Err(format!("{k} must be a list of strings")),
    }
}

impl Command {
    pub fn from_tool(name: &str, a: &Value) -> Result<Command, String> {
        let name = craft_biz::mcp::normalize_tool_name(name);
        Ok(match name.as_str() {
            "desk_status" => Command::Status,
            "desk_config" => Command::Config {
                support_address: opt_s(a, "support_address"),
                support_name: opt_s(a, "support_name"),
                signature: a
                    .get("signature")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
            "desk_ingest" => Command::Ingest {
                path: s(a, "path")?.into(),
                dry_run: flag(a, "dry_run"),
            },
            "desk_ticket_list" => Command::List {
                status: str_list(a, "status")?
                    .unwrap_or_default()
                    .iter()
                    .map(|x| Status::parse(x).map_err(e))
                    .collect::<Result<Vec<_>, _>>()?,
                priority: opt_s(a, "priority")
                    .map(|p| Priority::parse(&p).map_err(e))
                    .transpose()?,
                assignee: opt_s(a, "assignee"),
                text: opt_s(a, "text"),
                all: flag(a, "include_closed"),
            },
            "desk_ticket_show" => Command::Show {
                ticket: s(a, "ticket")?.into(),
            },
            "desk_ticket_update" => Command::Update {
                ticket: s(a, "ticket")?.into(),
                status: opt_s(a, "status")
                    .map(|x| Status::parse(&x).map_err(e))
                    .transpose()?,
                priority: opt_s(a, "priority")
                    .map(|x| Priority::parse(&x).map_err(e))
                    .transpose()?,
                assignee: match a.get("assignee") {
                    None => None,
                    Some(Value::Null) => Some(None),
                    Some(Value::String(x)) => Some(Some(x.clone())),
                    _ => return Err("assignee must be a string or null".into()),
                },
                tags: str_list(a, "tags")?,
                note: opt_s(a, "note"),
            },
            "desk_reply_draft" => Command::ReplyDraft {
                ticket: s(a, "ticket")?.into(),
                body: opt_s(a, "body"),
                macro_name: opt_s(a, "macro"),
            },
            "desk_reply_mark_sent" => Command::ReplyMarkSent {
                message: a
                    .get("message")
                    .and_then(Value::as_i64)
                    .ok_or("missing integer message (id from ticket_show)")?,
                at: opt_dt(a, "at")?,
            },
            "desk_macro_set" => Command::MacroSet {
                name: s(a, "name")?.into(),
                body: s(a, "body")?.into(),
                set_status: opt_s(a, "set_status")
                    .map(|x| Status::parse(&x).map_err(e))
                    .transpose()?,
                add_tags: str_list(a, "add_tags")?.unwrap_or_default(),
            },
            "desk_macro_list" => Command::MacroList,
            "desk_sla_status" => Command::Sla {
                as_of: opt_dt(a, "as_of")?,
            },
            "desk_escalate" => Command::Escalate {
                ticket: s(a, "ticket")?.into(),
                details: opt_s(a, "details").unwrap_or_default(),
                project: opt_s(a, "project"),
            },
            "desk_signals" => Command::Signals {
                emit: flag(a, "emit"),
            },
            "desk_demo_seed" => Command::DemoSeed,
            other => return Err(format!("unknown tool {other}")),
        })
    }
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|x| x.to_string())
}

fn support_address(d: &Desk) -> Result<Option<String>, String> {
    Ok(d.meta("support_address")
        .map_err(e)?
        .map(|a| a.to_lowercase()))
}

pub fn dispatch(ctx: &mut Ctx, cmd: Command) -> Result<Value, String> {
    match cmd {
        Command::Status => Ok(
            json!({ "app": APP, "version": VERSION, "stats": to_json(&ctx.desk.stats().map_err(e)?)?, "support_address": support_address(&ctx.desk)?, "outbox": ctx.outbox.display().to_string() }),
        ),
        Command::Config {
            support_address: addr,
            support_name,
            signature,
        } => {
            if let Some(a) = addr {
                if !a.contains('@') || a.contains(char::is_whitespace) {
                    return Err(format!("{a:?} is not an e-mail address"));
                }
                ctx.desk
                    .set_meta("support_address", &a.to_lowercase())
                    .map_err(e)?;
            }
            if let Some(n) = support_name {
                ctx.desk.set_meta("support_name", n.trim()).map_err(e)?;
            }
            if let Some(sig) = signature {
                ctx.desk.set_meta("signature", &sig).map_err(e)?;
            }
            Ok(
                json!({ "support_address": ctx.desk.meta("support_address").map_err(e)?, "support_name": ctx.desk.meta("support_name").map_err(e)?, "signature": ctx.desk.meta("signature").map_err(e)? }),
            )
        }
        Command::Ingest { path, dry_run } => ingest_path(ctx, &path, dry_run),
        Command::List {
            status,
            priority,
            assignee,
            text,
            all,
        } => {
            let st = if status.is_empty() && !all {
                vec![Status::New, Status::Open, Status::Pending]
            } else {
                status
            };
            let list = ctx
                .desk
                .tickets(&st, priority, assignee.as_deref(), text.as_deref())
                .map_err(e)?;
            Ok(json!({ "tickets": to_json(&list)? }))
        }
        Command::Show { ticket } => show(ctx, &ticket),
        Command::Update {
            ticket,
            status,
            priority,
            assignee,
            tags,
            note,
        } => update(ctx, &ticket, status, priority, assignee, tags, note),
        Command::ReplyDraft {
            ticket,
            body,
            macro_name,
        } => reply_draft(ctx, &ticket, body, macro_name),
        Command::ReplyMarkSent { message, at } => to_json(
            &ctx.desk
                .mark_sent(message, at.unwrap_or_else(now), &ctx.actor)
                .map_err(e)?,
        ),
        Command::MacroSet {
            name,
            body,
            set_status,
            add_tags,
        } => to_json(
            &ctx.desk
                .set_macro(&name, &body, set_status, &add_tags)
                .map_err(e)?,
        ),
        Command::MacroList => Ok(json!({ "macros": to_json(&ctx.desk.macros().map_err(e)?)? })),
        Command::Sla { as_of } => sla_status(&ctx.desk, as_of.unwrap_or_else(now)),
        Command::Escalate {
            ticket,
            details,
            project,
        } => escalate(ctx, &ticket, &details, project),
        Command::Signals { emit } => signals(ctx, emit),
        Command::DemoSeed => demo_seed(ctx),
    }
}

// ---------- ingest ----------

fn collect_files(root: &Path, out: &mut Vec<PathBuf>, depth: usize) -> Result<(), String> {
    if out.len() >= MAX_FILES {
        return Ok(());
    }
    if root.is_file() {
        out.push(root.to_path_buf());
        return Ok(());
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(root)
        .map_err(|x| format!("{}: {x}", root.display()))?
        .filter_map(|en| en.ok().map(|en| en.path()))
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            // Maildir: cur/new; also plain nested folders, three levels deep.
            if depth < 3 && p.file_name().is_some_and(|n| n != "tmp") {
                collect_files(&p, out, depth + 1)?;
            }
        } else {
            let in_maildir = p
                .parent()
                .and_then(|d| d.file_name())
                .is_some_and(|n| n == "cur" || n == "new");
            let is_eml = p.extension().is_some_and(|x| x.eq_ignore_ascii_case("eml"));
            if is_eml || in_maildir {
                out.push(p);
            }
        }
        if out.len() >= MAX_FILES {
            break;
        }
    }
    Ok(())
}

/// What happened to one message during ingest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    NewTicket(String),
    Appended(String),
    Outgoing(String),
    Duplicate,
    UnmatchedOutgoing,
}

fn ingest_path(ctx: &mut Ctx, path: &str, dry_run: bool) -> Result<Value, String> {
    let root = ctx.confinement.check(path, false)?;
    let mut files = Vec::new();
    collect_files(&root, &mut files, 0)?;
    let tx = ctx
        .desk
        .conn()
        .unchecked_transaction()
        .map_err(|x| x.to_string())?;
    let (mut new_tickets, mut appended, mut outgoing, mut dupes, mut unmatched) =
        (Vec::new(), 0, 0, 0, 0);
    let mut errors = Vec::new();
    for f in &files {
        let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        if size > MAX_MAIL_BYTES {
            errors.push(json!({ "file": f.display().to_string(), "error": "larger than 25 MB" }));
            continue;
        }
        let raw = match std::fs::read(f) {
            Ok(r) => r,
            Err(x) => {
                errors.push(json!({ "file": f.display().to_string(), "error": x.to_string() }));
                continue;
            }
        };
        match ingest_bytes(&ctx.desk, &raw, Some(&f.display().to_string())) {
            Ok(Outcome::NewTicket(k)) => new_tickets.push(k),
            Ok(Outcome::Appended(_)) => appended += 1,
            Ok(Outcome::Outgoing(_)) => outgoing += 1,
            Ok(Outcome::Duplicate) => dupes += 1,
            Ok(Outcome::UnmatchedOutgoing) => unmatched += 1,
            Err(x) => errors.push(json!({ "file": f.display().to_string(), "error": x })),
        }
    }
    if dry_run {
        drop(tx);
    } else {
        tx.commit().map_err(|x| x.to_string())?;
    }
    Ok(json!({
        "files": files.len(), "dry_run": dry_run, "new_tickets": new_tickets, "appended": appended,
        "outgoing": outgoing, "duplicates": dupes, "unmatched_outgoing": unmatched, "errors": errors,
        "limit_reached": files.len() >= MAX_FILES,
    }))
}

/// Thread one message into the desk. Same code path for files and the demo.
pub fn ingest_bytes(d: &Desk, raw: &[u8], source: Option<&str>) -> Result<Outcome, String> {
    let m = mail::parse(raw, source, now())?;
    if d.known_hash(&m.sha256).map_err(e)?
        || m.message_id
            .as_deref()
            .map(|id| d.known_message_id(id))
            .transpose()
            .map_err(e)?
            .unwrap_or(false)
    {
        return Ok(Outcome::Duplicate);
    }
    let ours = support_address(d)?.is_some_and(|a| a == m.from_email);
    let thread_ids: Vec<String> = m
        .in_reply_to
        .iter()
        .cloned()
        .chain(m.references.iter().cloned())
        .collect();
    let mut found = d.ticket_by_message_ids(&thread_ids).map_err(e)?;
    if found.is_none() {
        if let Some(n) = mail::ticket_tag(&m.subject) {
            found = d.ticket_by_id(n).ok();
        }
    }
    let normalized = mail::normalize_subject(&m.subject);
    if found.is_none() && !ours {
        found = d
            .ticket_by_subject(
                &m.from_email,
                &normalized,
                m.date - Duration::days(SUBJECT_WINDOW_DAYS),
            )
            .map_err(e)?;
    }
    if ours {
        let Some(t) = found.filter(|t| t.status != Status::Closed) else {
            return Ok(Outcome::UnmatchedOutgoing);
        };
        d.add_message(t.id, "out", &m, false, None).map_err(e)?;
        d.record_first_response(t.id, m.date).map_err(e)?;
        if t.status == Status::New {
            d.set_status(&t, Status::Open, "ingest", m.date)
                .map_err(e)?;
        }
        return Ok(Outcome::Outgoing(t.key));
    }
    match found {
        Some(t) if t.status != Status::Closed => {
            d.add_message(t.id, "in", &m, false, None).map_err(e)?;
            if matches!(t.status, Status::Pending | Status::Solved) {
                d.set_status(&t, Status::Open, "ingest", m.date)
                    .map_err(e)?;
            }
            Ok(Outcome::Appended(t.key))
        }
        closed => {
            let urgent_words = ["dringend", "urgent", "asap", "sofort", "notfall"];
            let lower = m.subject.to_lowercase();
            let prio = if urgent_words.iter().any(|w| lower.contains(w)) {
                Priority::High
            } else {
                Priority::Normal
            };
            let t = d
                .create_ticket(
                    &normalized,
                    &m.from_email,
                    m.from_name.as_deref(),
                    m.date,
                    prio,
                    "ingest",
                )
                .map_err(e)?;
            d.add_message(t.id, "in", &m, false, None).map_err(e)?;
            if let Some(c) = closed {
                let note = NewMessage {
                    from_email: "deskcraft".into(),
                    subject: "follow-up".into(),
                    date: m.date,
                    body: format!("Follow-up to closed ticket {}.", c.key),
                    ..Default::default()
                };
                d.add_message(t.id, "note", &note, false, None).map_err(e)?;
            }
            Ok(Outcome::NewTicket(t.key))
        }
    }
}

// ---------- tickets ----------

/// SLA clocks of one ticket (first response, resolution) as JSON.
pub fn sla_json(t: &Ticket, as_of: NaiveDateTime) -> Value {
    let (fr_target, res_target) = sla::targets(t.priority);
    let (fr_state, fr_used, fr_due) =
        sla::clock(t.opened_at, t.first_response_at, as_of, fr_target);
    let (res_state, res_used, res_due) = sla::clock(t.opened_at, t.solved_at, as_of, res_target);
    json!({
        "first_response": { "state": fr_state.as_str(), "used_minutes": fr_used, "target_minutes": fr_target, "due": sla::round_to_minute(fr_due) },
        "resolution": { "state": res_state.as_str(), "used_minutes": res_used, "target_minutes": res_target, "due": sla::round_to_minute(res_due) },
    })
}

fn show(ctx: &Ctx, key: &str) -> Result<Value, String> {
    let t = ctx.desk.ticket(key).map_err(e)?;
    let mut v = to_json(&t)?;
    v["messages"] = to_json(&ctx.desk.messages(t.id).map_err(e)?)?;
    v["history"] = to_json(&ctx.desk.history(t.id).map_err(e)?)?;
    v["sla"] = sla_json(&t, now());
    v["escalated"] = json!(ctx.desk.emitted(t.id, "ticket.escalated").map_err(e)?);
    Ok(v)
}

fn update(
    ctx: &mut Ctx,
    key: &str,
    status: Option<Status>,
    priority: Option<Priority>,
    assignee: Option<Option<String>>,
    tags: Option<Vec<String>>,
    note: Option<String>,
) -> Result<Value, String> {
    let d = &ctx.desk;
    let actor = ctx.actor.clone();
    let mut t = d.ticket(key).map_err(e)?;
    if let Some(p) = priority {
        t = d.set_priority(&t, p, &actor).map_err(e)?;
    }
    if let Some(a) = assignee {
        t = d.set_assignee(&t, a.as_deref(), &actor).map_err(e)?;
    }
    if let Some(tags) = tags {
        t = d.set_tags(&t, &tags, &actor).map_err(e)?;
    }
    if let Some(n) = note {
        let m = NewMessage {
            from_email: actor.clone(),
            subject: "internal note".into(),
            date: now(),
            body: n,
            ..Default::default()
        };
        d.add_message(t.id, "note", &m, false, None).map_err(e)?;
    }
    if let Some(s) = status {
        t = d.set_status(&t, s, &actor, now()).map_err(e)?;
    }
    to_json(&t)
}

fn fill(template: &str, t: &Ticket, agent: &str) -> String {
    let name = t
        .requester_name
        .clone()
        .unwrap_or_else(|| t.requester_email.clone());
    let first = name.split_whitespace().next().unwrap_or(&name).to_string();
    template
        .replace("{{name}}", &name)
        .replace("{{first_name}}", &first)
        .replace("{{ticket}}", &t.key)
        .replace("{{subject}}", &t.subject)
        .replace("{{agent}}", agent)
}

/// Write a reply as an .eml draft into the outbox. Nothing is sent: open the
/// file in your mail client (or send it with your SMTP tool), then mark it sent.
pub fn reply_draft(
    ctx: &mut Ctx,
    key: &str,
    body: Option<String>,
    macro_name: Option<String>,
) -> Result<Value, String> {
    let actor = ctx.actor.clone();
    let mut t = ctx.desk.ticket(key).map_err(e)?;
    if t.status == Status::Closed {
        return Err(format!("{} is closed", t.key));
    }
    let from = support_address(&ctx.desk)?
        .ok_or("set the support address first (desk_config support_address=...)")?;
    let (text, mac) = match (body, macro_name) {
        (Some(_), Some(_)) => return Err("pass body or macro, not both".into()),
        (None, None) => return Err("pass body or macro".into()),
        (Some(b), None) => (fill(&b, &t, &actor), None),
        (None, Some(m)) => {
            let mac = ctx.desk.macro_by_name(&m).map_err(e)?;
            (fill(&mac.body, &t, &actor), Some(mac))
        }
    };
    let signature = ctx.desk.meta("signature").map_err(e)?.unwrap_or_default();
    let full = if signature.trim().is_empty() {
        text
    } else {
        format!("{text}\n\n-- \n{signature}")
    };
    let messages = ctx.desk.messages(t.id).map_err(e)?;
    let last_in = messages.iter().rev().find(|m| m.direction == "in");
    let refs = ctx.desk.thread_ids(t.id).map_err(e)?;
    let id = format!("{}@deskcraft.invalid", uuid::Uuid::now_v7());
    let subject = format!("Re: {} [{}]", t.subject, t.key);
    let date = Local::now();
    let eml = mail::build_eml(&mail::Draft {
        message_id: &id,
        from_name: ctx.desk.meta("support_name").map_err(e)?.as_deref(),
        from: &from,
        to_name: t.requester_name.as_deref(),
        to: &t.requester_email,
        subject: &subject,
        in_reply_to: last_in.and_then(|m| m.message_id.as_deref()),
        references: &refs,
        date,
        ticket: &t.key,
        body: &full,
    });
    std::fs::create_dir_all(&ctx.outbox).map_err(|x| format!("{}: {x}", ctx.outbox.display()))?;
    let file = ctx.outbox.join(format!("{}-{}.eml", t.key, &id[..13]));
    std::fs::write(&file, &eml).map_err(|x| format!("{}: {x}", file.display()))?;
    let msg = NewMessage {
        message_id: Some(id),
        in_reply_to: last_in.and_then(|m| m.message_id.clone()),
        references: refs,
        from_email: from,
        from_name: None,
        to: t.requester_email.clone(),
        subject,
        date: date.naive_local(),
        body: full,
        sha256: mail::sha256_hex(eml.as_bytes()),
        source: None,
    };
    let stored = ctx
        .desk
        .add_message(t.id, "out", &msg, true, Some(&file.display().to_string()))
        .map_err(e)?
        .ok_or("draft already stored")?;
    if t.status == Status::New {
        t = ctx
            .desk
            .set_status(&t, Status::Open, &actor, now())
            .map_err(e)?;
    }
    if let Some(m) = mac {
        if !m.add_tags.is_empty() {
            let mut tags = t.tags.clone();
            tags.extend(m.add_tags.iter().cloned());
            t = ctx.desk.set_tags(&t, &tags, &actor).map_err(e)?;
        }
        if let Some(st) = m.set_status {
            t = ctx.desk.set_status(&t, st, &actor, now()).map_err(e)?;
        }
    }
    Ok(
        json!({ "ticket": t.key, "status": t.status, "message": stored.id, "file": file.display().to_string(), "sent": false, "next": "send it from your mail client, then desk_reply_mark_sent" }),
    )
}

pub fn sla_status(d: &Desk, as_of: NaiveDateTime) -> Result<Value, String> {
    let mut rows = Vec::new();
    let (mut breached, mut at_risk) = (0, 0);
    for t in d
        .tickets(
            &[Status::New, Status::Open, Status::Pending],
            None,
            None,
            None,
        )
        .map_err(e)?
    {
        let s = sla_json(&t, as_of);
        let states = [
            s["first_response"]["state"].as_str().unwrap_or(""),
            s["resolution"]["state"].as_str().unwrap_or(""),
        ];
        if states.contains(&"breached") {
            breached += 1;
        } else if states.contains(&"at_risk") {
            at_risk += 1;
        }
        rows.push(json!({
            "ticket": t.key, "priority": t.priority, "status": t.status, "subject": t.subject,
            "first_response": states[0], "first_response_due": s["first_response"]["due"],
            "resolution": states[1], "resolution_due": s["resolution"]["due"],
        }));
    }
    let rank = |r: &Value| match (r["first_response"].as_str(), r["resolution"].as_str()) {
        (Some("breached"), _) | (_, Some("breached")) => 0,
        (Some("at_risk"), _) | (_, Some("at_risk")) => 1,
        _ => 2,
    };
    rows.sort_by_key(rank);
    Ok(
        json!({ "as_of": sla::round_to_minute(as_of), "breached": breached, "at_risk": at_risk, "hours": "Mon-Fri 08:00-17:00, AT holidays excluded", "rows": rows }),
    )
}

/// Hand a ticket to engineering: `ticket.escalated` to taskcraft, once per ticket.
pub fn escalate(
    ctx: &mut Ctx,
    key: &str,
    details: &str,
    project: Option<String>,
) -> Result<Value, String> {
    let actor = ctx.actor.clone();
    let t = ctx.desk.ticket(key).map_err(e)?;
    if let Some(id) = ctx.desk.emitted(t.id, "ticket.escalated").map_err(e)? {
        return Err(format!("{} was already escalated (event {id})", t.key));
    }
    let last_in = ctx
        .desk
        .messages(t.id)
        .map_err(e)?
        .into_iter()
        .rev()
        .find(|m| m.direction == "in");
    let excerpt: String = last_in
        .map(|m| m.body.chars().take(1500).collect())
        .unwrap_or_default();
    let priority = match t.priority {
        Priority::Low => "low",
        Priority::Normal => "medium",
        Priority::High => "high",
        Priority::Urgent => "urgent",
    };
    let mut payload = json!({
        "ticket_id": t.key, "subject": t.subject, "priority": priority,
        "details": format!("{}{}Requester: {}\n\n{excerpt}", details.trim(), if details.trim().is_empty() { "" } else { "\n\n" }, t.requester_email),
    });
    if let Some(p) = project {
        payload["project"] = json!(p);
    }
    let ev = ctx
        .bus
        .emit(APP, "taskcraft", "ticket.escalated", payload)
        .map_err(|x| x.to_string())?;
    ctx.desk
        .record_emitted(t.id, "ticket.escalated", &ev.id, &actor)
        .map_err(e)?;
    let mut tags = t.tags.clone();
    tags.push("escalated".into());
    let t = ctx.desk.set_tags(&t, &tags, &actor).map_err(e)?;
    Ok(json!({ "ticket": t.key, "event": ev.id, "sent_to": "taskcraft" }))
}

/// Keyword score of a text: words from NEGATIVE count 1, STRONG count 2.
pub fn negativity(text: &str) -> (i64, Vec<&'static str>) {
    let lower = text.to_lowercase();
    let mut hits = Vec::new();
    let mut score = 0;
    for w in NEGATIVE {
        if lower.contains(w) {
            hits.push(*w);
            score += 1;
        }
    }
    for w in STRONG {
        if lower.contains(w) {
            hits.push(*w);
            score += 2;
        }
    }
    (score, hits)
}

/// Tickets whose customer mails read as unhappy (score >= 2). With `emit`, each
/// is sent once to gaincraft as `signal.negative` for the account health score.
pub fn signals(ctx: &mut Ctx, emit: bool) -> Result<Value, String> {
    let actor = ctx.actor.clone();
    let mut rows = Vec::new();
    let mut sent = 0;
    for t in ctx.desk.tickets(&[], None, None, None).map_err(e)? {
        let text: String = ctx
            .desk
            .messages(t.id)
            .map_err(e)?
            .iter()
            .filter(|m| m.direction == "in")
            .map(|m| format!("{} {}\n", m.subject, m.body))
            .collect();
        let (score, hits) = negativity(&text);
        if score < 2 {
            continue;
        }
        let mut event = ctx.desk.emitted(t.id, "signal.negative").map_err(e)?;
        if emit && event.is_none() {
            let domain = t
                .requester_email
                .split('@')
                .nth(1)
                .unwrap_or("")
                .to_string();
            let payload = json!({ "ticket_id": t.key, "account": domain, "email": t.requester_email, "subject": t.subject, "score": score, "hits": hits, "method": "keyword heuristic" });
            let ev = ctx
                .bus
                .emit(APP, "gaincraft", "signal.negative", payload)
                .map_err(|x| x.to_string())?;
            ctx.desk
                .record_emitted(t.id, "signal.negative", &ev.id, &actor)
                .map_err(e)?;
            sent += 1;
            event = Some(ev.id);
        }
        rows.push(json!({ "ticket": t.key, "requester": t.requester_email, "subject": t.subject, "score": score, "hits": hits, "sent_to_gaincraft": event }));
    }
    rows.sort_by_key(|r| -r["score"].as_i64().unwrap_or(0));
    Ok(
        json!({ "method": "deterministic DE/EN keyword heuristic, not sentiment AI", "emitted": sent, "rows": rows }),
    )
}

fn demo_mail(from: &str, subject: &str, date: &str, id: &str, extra: &str, body: &str) -> String {
    format!("From: {from}\r\nTo: support@mockdesk.at\r\nSubject: {subject}\r\nDate: {date}\r\nMessage-ID: <{id}>\r\n{extra}MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n{body}\r\n")
}

/// Labelled demo desk: real RFC 5322 mails through the normal ingest path.
pub fn demo_seed(ctx: &mut Ctx) -> Result<Value, String> {
    if !ctx
        .desk
        .tickets(&[], None, None, None)
        .map_err(e)?
        .is_empty()
    {
        return Err("demo seed only runs on an empty desk".into());
    }
    let d = &ctx.desk;
    d.set_meta("demo", "1").map_err(e)?;
    d.set_meta("support_address", "support@mockdesk.at")
        .map_err(e)?;
    d.set_meta("support_name", "Mockdesk Support (DEMO)")
        .map_err(e)?;
    d.set_meta("signature", "Mockdesk Support (DEMO)\nWien")
        .map_err(e)?;
    d.set_macro("geloest", "Hallo {{first_name}},\n\ndas Problem ({{subject}}) ist behoben. Bitte melden Sie sich, falls es wieder auftritt.\n\nLiebe Grüße\n{{agent}}", Some(Status::Solved), &["solved-macro".into()]).map_err(e)?;
    d.set_macro("rueckfrage", "Hallo {{first_name}},\n\ndanke für Ihre Nachricht zu {{ticket}}. Können Sie uns bitte einen Screenshot schicken?\n\nLiebe Grüße\n{{agent}}", Some(Status::Pending), &[]).map_err(e)?;
    let mails = [
        demo_mail("Joe Mocky <joe@mock-software.example>", "=?utf-8?Q?Drucker_im_B=C3=BCro_druckt_nicht?=", "Mon, 2 Nov 2026 09:15:00 +0100", "m1@mock-software.example", "", "Hallo,\r\nder Drucker im B=C3=BCro druckt seit heute nicht mehr. Fehlermeldung: Papierstau.\r\nLiebe Gr=C3=BC=C3=9Fe\r\nJoe"),
        demo_mail("Mockdesk Support <support@mockdesk.at>", "Re: Drucker im =?utf-8?Q?B=C3=BCro?= druckt nicht [D-1]", "Mon, 2 Nov 2026 10:05:00 +0100", "s1@mockdesk.at", "In-Reply-To: <m1@mock-software.example>\r\nReferences: <m1@mock-software.example>\r\n", "Hallo Joe,\r\nbitte die hintere Klappe =C3=B6ffnen und das Papier entfernen.\r\nMockdesk Support"),
        demo_mail("Joe Mocky <joe@mock-software.example>", "Re: Drucker im =?utf-8?Q?B=C3=BCro?= druckt nicht [D-1]", "Tue, 3 Nov 2026 08:30:00 +0100", "m2@mock-software.example", "In-Reply-To: <s1@mockdesk.at>\r\nReferences: <m1@mock-software.example> <s1@mockdesk.at>\r\n", "Danke, geht wieder. Aber jetzt druckt er nur grau."),
        demo_mail("Joe Mocky <joe@mock-software.example>", "AW: Drucker im =?utf-8?Q?B=C3=BCro?= druckt nicht", "Wed, 4 Nov 2026 14:00:00 +0100", "m3@mock-software.example", "", "Nachtrag ohne Reply-Header: Toner ist bestellt."),
        demo_mail("Mia Mockerl <mia@mockingbird.example>", "Invoice question", "Thu, 5 Nov 2026 16:40:00 +0100", "i1@mockingbird.example", "", "Hi, invoice 2026-118 shows 20% VAT but we are a company in Germany with a UID. Can you check?"),
        demo_mail("Hannes Mockinger <hannes@mockinger.example>", "Webshop schon wieder down - DRINGEND", "Fri, 6 Nov 2026 07:50:00 +0100", "w1@mockinger.example", "", "Das ist eine Frechheit, der Shop ist schon wieder offline. Ich bin sehr unzufrieden und =C3=BCberlege zu k=C3=BCndigen."),
    ];
    let mut outcomes = Vec::new();
    for (i, m) in mails.iter().enumerate() {
        let o = ingest_bytes(
            &ctx.desk,
            m.as_bytes(),
            Some(&format!("demo-{}.eml", i + 1)),
        )?;
        outcomes.push(format!("{o:?}"));
    }
    let t1 = ctx.desk.ticket("D-1").map_err(e)?;
    ctx.desk
        .set_assignee(&t1, Some("Anna Mockova"), "demo")
        .map_err(e)?;
    Ok(
        json!({ "demo": true, "ingest": outcomes, "stats": to_json(&ctx.desk.stats().map_err(e)?)? }),
    )
}

/// MCP tool catalog.
pub fn tools() -> Vec<Value> {
    let ticket = json!({ "type": "string", "description": "Ticket key like D-12" });
    let status =
        json!({ "type": "string", "enum": ["new", "open", "pending", "solved", "closed"] });
    let prio = json!({ "type": "string", "enum": ["low", "normal", "high", "urgent"] });
    let dt = json!({ "type": "string", "description": "YYYY-MM-DD HH:MM (default: now)" });
    vec![
        tool("desk_status", "Ticket counts by status, messages, unsent drafts, support address, outbox folder.", json!({}), &[]),
        tool("desk_config", "Show or set the support address (mails from it count as our replies), the sender name and the reply signature.", json!({ "support_address": {"type":"string"}, "support_name": {"type":"string"}, "signature": {"type":"string"} }), &[]),
        tool("desk_ingest", "Read .eml files or a maildir (cur/new) into tickets. Threads by Message-ID / In-Reply-To / References, then [D-n] subject tags, then same requester + subject within 30 days. Re-ingesting is a no-op. dry_run stores nothing.", json!({ "path": {"type":"string"}, "dry_run": {"type":"boolean"} }), &["path"]),
        tool("desk_ticket_list", "Tickets, urgent first. Default: new, open and pending.", json!({ "status": {"type":"array","items":status}, "priority": prio, "assignee": {"type":"string"}, "text": {"type":"string","description":"search subject, requester and message bodies"}, "include_closed": {"type":"boolean"} }), &[]),
        tool("desk_ticket_show", "One ticket with all messages, internal notes, history and SLA clocks.", json!({ "ticket": ticket }), &["ticket"]),
        tool("desk_ticket_update", "Change status, priority, assignee (null clears), tags (replaces the set) or add an internal note.", json!({ "ticket": ticket, "status": status, "priority": prio, "assignee": {"type":["string","null"]}, "tags": {"type":"array","items":{"type":"string"}}, "note": {"type":"string"} }), &["ticket"]),
        tool("desk_reply_draft", "Write a reply as an .eml draft into the outbox (threaded headers, [D-n] in the subject). Nothing is sent. body or macro; {{name}}, {{first_name}}, {{ticket}}, {{subject}}, {{agent}} are filled.", json!({ "ticket": ticket, "body": {"type":"string"}, "macro": {"type":"string"} }), &["ticket"]),
        tool("desk_reply_mark_sent", "Mark a reply draft as sent (after sending it from your mail client). The first sent reply stops the first-response clock.", json!({ "message": {"type":"integer"}, "at": dt }), &["message"]),
        tool("desk_macro_set", "Create or change a reply macro, optionally setting a status and adding tags when used.", json!({ "name": {"type":"string"}, "body": {"type":"string"}, "set_status": status, "add_tags": {"type":"array","items":{"type":"string"}} }), &["name", "body"]),
        tool("desk_macro_list", "Reply macros.", json!({}), &[]),
        tool("desk_sla_status", "First-response and resolution clocks of open tickets in business hours (Mon-Fri 08:00-17:00, AT holidays excluded): met, ok, at_risk, breached.", json!({ "as_of": dt }), &[]),
        tool("desk_escalate", "Send a ticket to taskcraft as ticket.escalated (becomes an issue there). Once per ticket; tags it escalated.", json!({ "ticket": ticket, "details": {"type":"string"}, "project": {"type":"string","description":"taskcraft project key"} }), &["ticket"]),
        tool("desk_signals", "Tickets whose customer mails read as unhappy (deterministic DE/EN keywords, not AI). emit=true sends signal.negative to gaincraft once per ticket.", json!({ "emit": {"type":"boolean"} }), &[]),
        tool("desk_demo_seed", "Fill an EMPTY desk with labelled demo mails (MIME encoded, threaded, one angry customer) and macros.", json!({}), &[]),
    ]
}

#[cfg(test)]
mod tests;
