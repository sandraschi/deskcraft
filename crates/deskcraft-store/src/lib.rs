//! deskcraft-store: support tickets built from e-mail.
//!
//! Invariants:
//! - A message is stored once: Message-ID is unique, and so is the SHA-256 of
//!   the raw bytes (for mail without a Message-ID). Re-ingesting is a no-op.
//! - Tickets are numbered `D-n`, never reused.
//! - Every ticket change writes a history row (actor, field, old, new).
//! - Status moves: new/open/pending/solved can move freely among themselves;
//!   closed is final (a reply to a closed ticket opens a new ticket).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{NaiveDateTime, Utc};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::path::Path;

pub const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum DeskError {
    #[error("database: {0}")]
    Db(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("not allowed: {0}")]
    State(String),
}

impl From<rusqlite::Error> for DeskError {
    fn from(e: rusqlite::Error) -> Self {
        DeskError::Db(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, DeskError>;

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

macro_rules! str_enum {
    ($name:ident { $($var:ident => $s:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($var),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),+];
            pub fn as_str(self) -> &'static str { match self { $($name::$var => $s),+ } }
            pub fn parse(s: &str) -> Result<$name> {
                let t = s.trim().to_ascii_lowercase().replace([' ', '-'], "_");
                $name::ALL.iter().copied().find(|v| v.as_str() == t).ok_or_else(|| {
                    DeskError::Invalid(format!("unknown {} {s:?} (one of: {})", stringify!($name).to_lowercase(),
                        $name::ALL.iter().map(|v| v.as_str()).collect::<Vec<_>>().join(", ")))
                })
            }
        }
    };
}

str_enum!(Status { New => "new", Open => "open", Pending => "pending", Solved => "solved", Closed => "closed" });
str_enum!(Priority { Low => "low", Normal => "normal", High => "high", Urgent => "urgent" });

#[derive(Debug, Clone, Serialize)]
pub struct Ticket {
    pub id: i64,
    pub key: String,
    pub subject: String,
    pub requester_email: String,
    pub requester_name: Option<String>,
    pub status: Status,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub tags: Vec<String>,
    /// Local time of the first customer message.
    pub opened_at: NaiveDateTime,
    pub first_response_at: Option<NaiveDateTime>,
    pub solved_at: Option<NaiveDateTime>,
    pub updated_at: String,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: i64,
    pub ticket: String,
    pub direction: String,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub from_email: String,
    pub from_name: Option<String>,
    pub to: String,
    pub subject: String,
    pub date: NaiveDateTime,
    pub body: String,
    pub draft: bool,
    pub sent_at: Option<NaiveDateTime>,
    pub file: Option<String>,
    pub internal: bool,
}

/// A parsed message ready for storage.
#[derive(Debug, Clone, Default)]
pub struct NewMessage {
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub from_email: String,
    pub from_name: Option<String>,
    pub to: String,
    pub subject: String,
    pub date: NaiveDateTime,
    pub body: String,
    pub sha256: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Macro {
    pub name: String,
    pub body: String,
    pub set_status: Option<Status>,
    pub add_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryRow {
    pub at: String,
    pub actor: String,
    pub field: String,
    pub old: Option<String>,
    pub new: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub by_status: std::collections::BTreeMap<String, i64>,
    pub messages: i64,
    pub drafts_unsent: i64,
    pub demo: bool,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS tickets (
    id INTEGER PRIMARY KEY,
    subject TEXT NOT NULL,
    requester_email TEXT NOT NULL,
    requester_name TEXT,
    status TEXT NOT NULL,
    priority TEXT NOT NULL,
    assignee TEXT,
    tags TEXT NOT NULL DEFAULT '[]',
    opened_at TEXT NOT NULL,
    first_response_at TEXT,
    solved_at TEXT,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    ticket_id INTEGER NOT NULL REFERENCES tickets(id),
    direction TEXT NOT NULL CHECK (direction IN ('in', 'out', 'note')),
    message_id TEXT UNIQUE,
    in_reply_to TEXT,
    refs TEXT NOT NULL DEFAULT '[]',
    from_email TEXT NOT NULL,
    from_name TEXT,
    to_addr TEXT NOT NULL DEFAULT '',
    subject TEXT NOT NULL,
    date TEXT NOT NULL,
    body TEXT NOT NULL,
    sha256 TEXT UNIQUE,
    source TEXT,
    draft INTEGER NOT NULL DEFAULT 0,
    sent_at TEXT,
    file TEXT
);
CREATE INDEX IF NOT EXISTS idx_messages_ticket ON messages(ticket_id, date);
CREATE TABLE IF NOT EXISTS history (
    id INTEGER PRIMARY KEY,
    ticket_id INTEGER NOT NULL REFERENCES tickets(id),
    at TEXT NOT NULL,
    actor TEXT NOT NULL,
    field TEXT NOT NULL,
    old TEXT,
    new TEXT
);
CREATE TABLE IF NOT EXISTS macros (
    name TEXT PRIMARY KEY,
    body TEXT NOT NULL,
    set_status TEXT,
    add_tags TEXT NOT NULL DEFAULT '[]'
);
CREATE TABLE IF NOT EXISTS emitted (
    ticket_id INTEGER NOT NULL REFERENCES tickets(id),
    kind TEXT NOT NULL,
    event_id TEXT NOT NULL,
    at TEXT NOT NULL,
    PRIMARY KEY (ticket_id, kind)
);
";

const TICKET_COLS: &str = "t.id, t.subject, t.requester_email, t.requester_name, t.status, t.priority, t.assignee, t.tags, t.opened_at, t.first_response_at, t.solved_at, t.updated_at,
    (SELECT COUNT(*) FROM messages m WHERE m.ticket_id = t.id AND m.direction <> 'note')";
const MSG_COLS: &str = "m.id, m.ticket_id, m.direction, m.message_id, m.in_reply_to, m.from_email, m.from_name, m.to_addr, m.subject, m.date, m.body, m.draft, m.sent_at, m.file";

type TicketRaw = (
    i64,
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    String,
    NaiveDateTime,
    Option<NaiveDateTime>,
    Option<NaiveDateTime>,
    String,
    i64,
);

fn ticket_raw(r: &Row) -> rusqlite::Result<TicketRaw> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
        r.get(11)?,
        r.get(12)?,
    ))
}

fn ticket_from(raw: TicketRaw) -> Result<Ticket> {
    let (
        id,
        subject,
        requester_email,
        requester_name,
        status,
        priority,
        assignee,
        tags,
        opened_at,
        first_response_at,
        solved_at,
        updated_at,
        message_count,
    ) = raw;
    Ok(Ticket {
        id,
        key: format!("D-{id}"),
        subject,
        requester_email,
        requester_name,
        status: Status::parse(&status)?,
        priority: Priority::parse(&priority)?,
        assignee,
        tags: serde_json::from_str(&tags)
            .map_err(|e| DeskError::Db(format!("tags of D-{id}: {e}")))?,
        opened_at,
        first_response_at,
        solved_at,
        updated_at,
        message_count,
    })
}

fn msg_row(r: &Row) -> rusqlite::Result<Message> {
    let direction: String = r.get(2)?;
    Ok(Message {
        id: r.get(0)?,
        ticket: format!("D-{}", r.get::<_, i64>(1)?),
        internal: direction == "note",
        direction,
        message_id: r.get(3)?,
        in_reply_to: r.get(4)?,
        from_email: r.get(5)?,
        from_name: r.get(6)?,
        to: r.get(7)?,
        subject: r.get(8)?,
        date: r.get(9)?,
        body: r.get(10)?,
        draft: r.get(11)?,
        sent_at: r.get(12)?,
        file: r.get(13)?,
    })
}

/// `D-12`, `d12`, `#12` or `12`.
pub fn parse_key(key: &str) -> Result<i64> {
    let t = key.trim().trim_start_matches('#');
    let digits = t
        .strip_prefix("D-")
        .or_else(|| t.strip_prefix("d-"))
        .or_else(|| t.strip_prefix('D'))
        .or_else(|| t.strip_prefix('d'))
        .unwrap_or(t);
    digits
        .parse::<i64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| DeskError::Invalid(format!("ticket key {key:?} (use D-12)")))
}

fn clean_tags(tags: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = t.trim().to_lowercase();
        if t.is_empty() {
            continue;
        }
        if t.len() > 40
            || !t
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(DeskError::Invalid(format!(
                "tag {t:?}: letters, digits, - and _ only, max 40"
            )));
        }
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out.sort();
    Ok(out)
}

pub struct Desk {
    conn: Connection,
}

impl Desk {
    pub fn open(path: &Path) -> Result<Desk> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir).map_err(|e| DeskError::Db(e.to_string()))?;
            }
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Desk> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Desk> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
        conn.execute_batch(SCHEMA)?;
        let v: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        match v.as_deref() {
            None => {
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
                    params![SCHEMA_VERSION.to_string()],
                )?;
            }
            Some(x) if x == SCHEMA_VERSION.to_string() => {}
            Some(x) => {
                return Err(DeskError::Db(format!(
                    "database schema {x} is newer than this build ({SCHEMA_VERSION})"
                )))
            }
        }
        Ok(Desk { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])?;
        Ok(())
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn is_demo(&self) -> Result<bool> {
        Ok(self.meta("demo")?.as_deref() == Some("1"))
    }

    // ---------- lookups for threading ----------

    pub fn known_hash(&self, sha256: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM messages WHERE sha256 = ?1)",
            params![sha256],
            |r| r.get(0),
        )?)
    }

    pub fn known_message_id(&self, message_id: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM messages WHERE message_id = ?1)",
            params![message_id],
            |r| r.get(0),
        )?)
    }

    /// Ticket of the first of `ids` that we already store.
    pub fn ticket_by_message_ids(&self, ids: &[String]) -> Result<Option<Ticket>> {
        for id in ids {
            let t: Option<i64> = self
                .conn
                .query_row(
                    "SELECT ticket_id FROM messages WHERE message_id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(t) = t {
                return self.ticket_by_id(t).map(Some);
            }
        }
        Ok(None)
    }

    pub fn ticket_by_id(&self, id: i64) -> Result<Ticket> {
        let raw = self
            .conn
            .query_row(
                &format!("SELECT {TICKET_COLS} FROM tickets t WHERE t.id = ?1"),
                params![id],
                ticket_raw,
            )
            .optional()?
            .ok_or_else(|| DeskError::NotFound(format!("ticket D-{id}")))?;
        ticket_from(raw)
    }

    pub fn ticket(&self, key: &str) -> Result<Ticket> {
        self.ticket_by_id(parse_key(key)?)
    }

    /// Open ticket of this requester whose normalized subject matches, opened after `since`.
    pub fn ticket_by_subject(
        &self,
        requester: &str,
        normalized_subject: &str,
        since: NaiveDateTime,
    ) -> Result<Option<Ticket>> {
        let raw = self
            .conn
            .query_row(
                &format!("SELECT {TICKET_COLS} FROM tickets t WHERE lower(t.requester_email) = lower(?1) AND lower(t.subject) = lower(?2) AND t.status <> 'closed' AND t.opened_at >= ?3 ORDER BY t.id DESC LIMIT 1"),
                params![requester, normalized_subject, since],
                ticket_raw,
            )
            .optional()?;
        raw.map(ticket_from).transpose()
    }

    // ---------- writes ----------

    pub fn create_ticket(
        &self,
        subject: &str,
        requester_email: &str,
        requester_name: Option<&str>,
        opened_at: NaiveDateTime,
        priority: Priority,
        actor: &str,
    ) -> Result<Ticket> {
        let subject = if subject.trim().is_empty() {
            "(no subject)"
        } else {
            subject.trim()
        };
        let stamp = now();
        self.conn.execute(
            "INSERT INTO tickets (subject, requester_email, requester_name, status, priority, opened_at, updated_at) VALUES (?1, ?2, ?3, 'new', ?4, ?5, ?6)",
            params![subject, requester_email.trim().to_lowercase(), requester_name, priority.as_str(), opened_at, stamp],
        )?;
        let id = self.conn.last_insert_rowid();
        self.log(id, actor, "created", None, Some(&format!("D-{id}")))?;
        self.ticket_by_id(id)
    }

    /// Store a message on a ticket. Returns None when it is already stored.
    pub fn add_message(
        &self,
        ticket_id: i64,
        direction: &str,
        m: &NewMessage,
        draft: bool,
        file: Option<&str>,
    ) -> Result<Option<Message>> {
        if let Some(id) = &m.message_id {
            if self.known_message_id(id)? {
                return Ok(None);
            }
        }
        if !m.sha256.is_empty() && self.known_hash(&m.sha256)? {
            return Ok(None);
        }
        let refs =
            serde_json::to_string(&m.references).map_err(|e| DeskError::Invalid(e.to_string()))?;
        self.conn.execute(
            "INSERT INTO messages (ticket_id, direction, message_id, in_reply_to, refs, from_email, from_name, to_addr, subject, date, body, sha256, source, draft, file)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                ticket_id,
                direction,
                m.message_id,
                m.in_reply_to,
                refs,
                m.from_email.trim().to_lowercase(),
                m.from_name,
                m.to,
                m.subject,
                m.date,
                m.body,
                (!m.sha256.is_empty()).then_some(&m.sha256),
                m.source,
                draft,
                file
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.touch(ticket_id)?;
        self.message(id).map(Some)
    }

    pub fn message(&self, id: i64) -> Result<Message> {
        self.conn
            .query_row(
                &format!("SELECT {MSG_COLS} FROM messages m WHERE m.id = ?1"),
                params![id],
                msg_row,
            )
            .optional()?
            .ok_or_else(|| DeskError::NotFound(format!("message {id}")))
    }

    pub fn messages(&self, ticket_id: i64) -> Result<Vec<Message>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {MSG_COLS} FROM messages m WHERE m.ticket_id = ?1 ORDER BY m.date, m.id"
        ))?;
        let rows = st
            .query_map(params![ticket_id], msg_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// All Message-IDs of a ticket in date order (for References headers).
    pub fn thread_ids(&self, ticket_id: i64) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT message_id FROM messages WHERE ticket_id = ?1 AND message_id IS NOT NULL AND draft = 0 AND direction <> 'note' ORDER BY date, id")?;
        let rows = st
            .query_map(params![ticket_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }

    fn touch(&self, ticket_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE tickets SET updated_at = ?2 WHERE id = ?1",
            params![ticket_id, now()],
        )?;
        Ok(())
    }

    fn log(
        &self,
        ticket_id: i64,
        actor: &str,
        field: &str,
        old: Option<&str>,
        new: Option<&str>,
    ) -> Result<()> {
        self.conn.execute("INSERT INTO history (ticket_id, at, actor, field, old, new) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![ticket_id, now(), actor, field, old, new])?;
        Ok(())
    }

    pub fn history(&self, ticket_id: i64) -> Result<Vec<HistoryRow>> {
        let mut st = self.conn.prepare(
            "SELECT at, actor, field, old, new FROM history WHERE ticket_id = ?1 ORDER BY id",
        )?;
        let rows = st
            .query_map(params![ticket_id], |r| {
                Ok(HistoryRow {
                    at: r.get(0)?,
                    actor: r.get(1)?,
                    field: r.get(2)?,
                    old: r.get(3)?,
                    new: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_status(
        &self,
        t: &Ticket,
        to: Status,
        actor: &str,
        at: NaiveDateTime,
    ) -> Result<Ticket> {
        if t.status == to {
            return Ok(t.clone());
        }
        if t.status == Status::Closed {
            return Err(DeskError::State(format!(
                "{} is closed; a new message opens a new ticket",
                t.key
            )));
        }
        let solved = if to == Status::Solved { Some(at) } else { None };
        self.conn.execute(
            "UPDATE tickets SET status = ?2, solved_at = ?3, updated_at = ?4 WHERE id = ?1",
            params![t.id, to.as_str(), solved, now()],
        )?;
        self.log(
            t.id,
            actor,
            "status",
            Some(t.status.as_str()),
            Some(to.as_str()),
        )?;
        self.ticket_by_id(t.id)
    }

    pub fn set_priority(&self, t: &Ticket, p: Priority, actor: &str) -> Result<Ticket> {
        if t.priority == p {
            return Ok(t.clone());
        }
        self.conn.execute(
            "UPDATE tickets SET priority = ?2, updated_at = ?3 WHERE id = ?1",
            params![t.id, p.as_str(), now()],
        )?;
        self.log(
            t.id,
            actor,
            "priority",
            Some(t.priority.as_str()),
            Some(p.as_str()),
        )?;
        self.ticket_by_id(t.id)
    }

    pub fn set_assignee(&self, t: &Ticket, who: Option<&str>, actor: &str) -> Result<Ticket> {
        let who = who.map(str::trim).filter(|w| !w.is_empty());
        if t.assignee.as_deref() == who {
            return Ok(t.clone());
        }
        self.conn.execute(
            "UPDATE tickets SET assignee = ?2, updated_at = ?3 WHERE id = ?1",
            params![t.id, who, now()],
        )?;
        self.log(t.id, actor, "assignee", t.assignee.as_deref(), who)?;
        self.ticket_by_id(t.id)
    }

    pub fn set_tags(&self, t: &Ticket, tags: &[String], actor: &str) -> Result<Ticket> {
        let tags = clean_tags(tags)?;
        if tags == t.tags {
            return Ok(t.clone());
        }
        let json = serde_json::to_string(&tags).map_err(|e| DeskError::Invalid(e.to_string()))?;
        self.conn.execute(
            "UPDATE tickets SET tags = ?2, updated_at = ?3 WHERE id = ?1",
            params![t.id, json, now()],
        )?;
        self.log(
            t.id,
            actor,
            "tags",
            Some(&t.tags.join(",")),
            Some(&tags.join(",")),
        )?;
        self.ticket_by_id(t.id)
    }

    /// First response is the first reply that was actually sent.
    pub fn mark_sent(&self, message_id: i64, at: NaiveDateTime, actor: &str) -> Result<Message> {
        let m = self.message(message_id)?;
        if m.direction != "out" || !m.draft {
            return Err(DeskError::State(format!(
                "message {message_id} is not an unsent reply draft"
            )));
        }
        self.conn.execute(
            "UPDATE messages SET draft = 0, sent_at = ?2, date = ?2 WHERE id = ?1",
            params![message_id, at],
        )?;
        let tid = parse_key(&m.ticket)?;
        self.conn.execute("UPDATE tickets SET first_response_at = COALESCE(first_response_at, ?2), updated_at = ?3 WHERE id = ?1", params![tid, at, now()])?;
        self.log(
            tid,
            actor,
            "reply_sent",
            None,
            Some(&message_id.to_string()),
        )?;
        self.message(message_id)
    }

    /// An outgoing message found by ingest (e.g. from the Sent folder) counts as sent.
    pub fn record_first_response(&self, ticket_id: i64, at: NaiveDateTime) -> Result<()> {
        self.conn.execute(
            "UPDATE tickets SET first_response_at = COALESCE(first_response_at, ?2) WHERE id = ?1",
            params![ticket_id, at],
        )?;
        Ok(())
    }

    pub fn tickets(
        &self,
        status: &[Status],
        priority: Option<Priority>,
        assignee: Option<&str>,
        text: Option<&str>,
    ) -> Result<Vec<Ticket>> {
        let mut sql = format!("SELECT {TICKET_COLS} FROM tickets t WHERE 1 = 1");
        let mut args: Vec<SqlValue> = Vec::new();
        if !status.is_empty() {
            let marks: Vec<String> = status
                .iter()
                .map(|s| {
                    args.push(SqlValue::Text(s.as_str().into()));
                    format!("?{}", args.len())
                })
                .collect();
            sql.push_str(&format!(" AND t.status IN ({})", marks.join(", ")));
        }
        if let Some(p) = priority {
            args.push(SqlValue::Text(p.as_str().into()));
            sql.push_str(&format!(" AND t.priority = ?{}", args.len()));
        }
        if let Some(a) = assignee {
            args.push(SqlValue::Text(a.trim().to_lowercase()));
            sql.push_str(&format!(
                " AND lower(COALESCE(t.assignee, '')) = ?{}",
                args.len()
            ));
        }
        if let Some(q) = text.map(str::trim).filter(|q| !q.is_empty()) {
            args.push(SqlValue::Text(format!("%{}%", q.to_lowercase())));
            let i = args.len();
            sql.push_str(&format!(" AND (lower(t.subject) LIKE ?{i} OR lower(t.requester_email) LIKE ?{i} OR EXISTS (SELECT 1 FROM messages m WHERE m.ticket_id = t.id AND lower(m.body) LIKE ?{i}))"));
        }
        sql.push_str(" ORDER BY CASE t.priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 WHEN 'normal' THEN 2 ELSE 3 END, t.opened_at");
        let mut st = self.conn.prepare(&sql)?;
        let raws = st
            .query_map(params_from_iter(args.iter()), ticket_raw)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        raws.into_iter().map(ticket_from).collect()
    }

    // ---------- macros ----------

    pub fn set_macro(
        &self,
        name: &str,
        body: &str,
        set_status: Option<Status>,
        add_tags: &[String],
    ) -> Result<Macro> {
        let n = name.trim().to_lowercase();
        if n.is_empty()
            || n.len() > 60
            || !n
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(DeskError::Invalid(
                "macro name: letters, digits, - and _".into(),
            ));
        }
        if body.trim().is_empty() {
            return Err(DeskError::Invalid("macro body is empty".into()));
        }
        let tags = clean_tags(add_tags)?;
        self.conn.execute(
            "INSERT INTO macros (name, body, set_status, add_tags) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET body = excluded.body, set_status = excluded.set_status, add_tags = excluded.add_tags",
            params![n, body, set_status.map(Status::as_str), serde_json::to_string(&tags).map_err(|e| DeskError::Invalid(e.to_string()))?],
        )?;
        self.macro_by_name(&n)
    }

    pub fn macro_by_name(&self, name: &str) -> Result<Macro> {
        let raw: Option<(String, String, Option<String>, String)> = self
            .conn
            .query_row(
                "SELECT name, body, set_status, add_tags FROM macros WHERE name = ?1",
                params![name.trim().to_lowercase()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let (name, body, st, tags) =
            raw.ok_or_else(|| DeskError::NotFound(format!("macro {name}")))?;
        Ok(Macro {
            name,
            body,
            set_status: st.map(|s| Status::parse(&s)).transpose()?,
            add_tags: serde_json::from_str(&tags).map_err(|e| DeskError::Db(e.to_string()))?,
        })
    }

    pub fn macros(&self) -> Result<Vec<Macro>> {
        let names: Vec<String> = {
            let mut st = self.conn.prepare("SELECT name FROM macros ORDER BY name")?;
            let v = st
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            v
        };
        names.iter().map(|n| self.macro_by_name(n)).collect()
    }

    // ---------- events ----------

    pub fn emitted(&self, ticket_id: i64, kind: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT event_id FROM emitted WHERE ticket_id = ?1 AND kind = ?2",
                params![ticket_id, kind],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn record_emitted(
        &self,
        ticket_id: i64,
        kind: &str,
        event_id: &str,
        actor: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO emitted (ticket_id, kind, event_id, at) VALUES (?1, ?2, ?3, ?4)",
            params![ticket_id, kind, event_id, now()],
        )?;
        self.log(ticket_id, actor, kind, None, Some(event_id))
    }

    pub fn stats(&self) -> Result<Stats> {
        let mut by = std::collections::BTreeMap::new();
        let mut st = self
            .conn
            .prepare("SELECT status, COUNT(*) FROM tickets GROUP BY status")?;
        for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (k, v) = row?;
            by.insert(k, v);
        }
        Ok(Stats {
            by_status: by,
            messages: self.conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE direction <> 'note'",
                [],
                |r| r.get(0),
            )?,
            drafts_unsent: self.conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE draft = 1",
                [],
                |r| r.get(0),
            )?,
            demo: self.is_demo()?,
        })
    }
}

#[cfg(test)]
mod tests;
