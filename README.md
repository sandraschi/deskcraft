# deskcraft

![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)
![Rust 1.90+](https://img.shields.io/badge/rust-1.90%2B-orange?logo=rust)
![MCP stdio](https://img.shields.io/badge/MCP-stdio-purple)
![ArtCraft harness](https://img.shields.io/badge/ArtCraft-harness-green)

Support tickets built from e-mail for a small business: threading, business-hours SLA, reply drafts as `.eml`, macros, escalation to taskcraft and unhappy-customer signals to gaincraft. One SQLite file, one binary that is both CLI and MCP server. Part of the back-office craft suite (replaces Zendesk / Freshdesk / a shared support inbox).

Status: **v0.2.0, Tier 2** (store + CLI + MCP + desktop window, tested). Synthetic demo mail only. **Nothing is ever sent by deskcraft**: replies are `.eml` drafts you send from your mail client.

- How to use it day to day: [HELP.md](HELP.md)
- What it is for and where it stops: [docs/PRD.md](docs/PRD.md)
- Every MCP tool: [docs/TOOLS.md](docs/TOOLS.md) (generated, checked by a test)
- Changes: [CHANGELOG.md](CHANGELOG.md)

## What it does

- **Ingest** `.eml` files or a maildir (`cur/`, `new/`), offline. MIME, quoted-printable, base64 and RFC 2047 headers are decoded by `mail-parser`. Re-ingesting the same folder is a no-op (Message-ID and content hash are unique). `--dry-run` shows what would happen.
- **Threading** in this order: Message-ID / In-Reply-To / References, then the `[D-12]` tag deskcraft puts into reply subjects, then same requester + same subject (Re/AW/WG stripped) within 30 days. Mail from your support address counts as your reply and stops the first-response clock. A reply to a closed ticket opens a new ticket with a note.
- **Tickets:** status new / open / pending / solved / closed, priority low..urgent ("dringend", "urgent" in the subject raise it), assignee, tags, internal notes, full history with actor.
- **SLA** in business hours (Mon-Fri 08:00-17:00, Austrian public holidays excluded): first response and resolution per priority, states met / ok / at_risk / breached with due times.
- **Replies:** `.eml` drafts with correct In-Reply-To / References, UTF-8 subject encoding, signature; macros fill `{{first_name}}`, `{{ticket}}`, `{{subject}}` and can set a status and tags.
- **Escalation:** `ticket.escalated` to taskcraft (becomes an issue there), once per ticket.
- **Signals:** deterministic DE/EN keyword heuristic (unzufrieden, kündigen, Frechheit, refund, ...) finds unhappy customers; `--emit` sends `signal.negative` to gaincraft, once per ticket.

## Desktop window

`deskcraft-app` is laid out like a help desk:

- **Views** on the left with live counts: all unsolved, unassigned, urgent and high, SLA at risk, pending, recently solved.
- **Ticket table** with status dot, priority, status, SLA badge (due time, at risk, breached), assignee, tags.
- **Ticket page:** properties on the left (status, priority, assignee with "Take it", tags, both SLA clocks as bars, escalate to taskcraft, history), the conversation in the middle (customer, replies, drafts, internal notes), a reply composer with macro picker and an internal note box. Drafts show "Mark as sent" and "Show in folder".
- **Board** by status: drag to change it, optional swimlanes by priority. Dropping on New is refused (only mail makes a ticket new).
- **Unhappy customers** with the matched keywords and "Send to gaincraft".
- **Macros**, **Ingest mail** (folder or file picker, dry run), **Settings** (support address, sender, signature).

Every change goes through the same command path as CLI and MCP. Ctrl+K lists every tool with a form, and a test fails if a tool has no place in the window. High-contrast light and dark, Ctrl+mouse wheel zoom, German and English.

```powershell
deskcraft-app --demo                 # on an empty desk: load the demo first
deskcraft-app --select D-3           # open a ticket directly
deskcraft-app --dark --lang en --view 6
```

## Quick start

```powershell
deskcraft demo-seed
deskcraft list
deskcraft show D-1
deskcraft sla
deskcraft reply D-2 --macro geloest
deskcraft escalate D-3 --project OPS
deskcraft signals
```

Your own mail: `deskcraft config --address support@yourfirm.at --name "Support"`, export the support folder as `.eml` (Outlook: drag mails into a folder; Thunderbird: ImportExportTools NG) and `deskcraft ingest D:\Mail\Support`.

Database: `--db PATH`, else `$DESKCRAFT_DB`, else `%LOCALAPPDATA%\deskcraft\deskcraft.db`. Outbox: `--outbox`, else `outbox` next to the database.

## MCP

```json
{ "mcpServers": { "deskcraft": { "command": "D:/Dev/cargo-target/release/deskcraft.exe", "args": ["mcp", "--allow-path", "D:/Mail/Support"] } } }
```

14 tools, all `desk_*`; see [docs/TOOLS.md](docs/TOOLS.md). Agent actions are recorded with actor `agent`. Without `--allow-path` the server cannot read any mail folder.

## Build

```powershell
just ci           # clippy -D warnings, tests (incl. docs/TOOLS.md sync), fmt check
just docs         # regenerate docs/TOOLS.md after changing a tool
just build-release
```

Needs `../craft-biz` checked out next to this repo.

## Not done (honest list)

See the "Gaps" section of [docs/PRD.md](docs/PRD.md). In short: no IMAP, no sending, pending does not pause the SLA clock, no attachments stored.
