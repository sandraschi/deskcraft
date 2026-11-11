# deskcraft PRD

## What it is

A local-first support desk for a 1-10 person business: customer e-mails become tickets, replies are drafted with threading intact, service levels are measured in business hours, and problems flow on to engineering (taskcraft) and account health (gaincraft). Pure Rust, ArtCraft pattern: one engine, CLI + MCP + desktop window.

Part of the back-office craft suite: `mcp-central-docs/projects/craft-fleet-expansion/BACKOFFICE_CRAFT_SUITE.md`, section 4.6.

## Users

- The person answering support mail (often the owner). Uses the desktop window or the CLI.
- Agents over MCP: triage (priority, tags, assignee), draft replies from macros, watch the SLA, escalate. Every agent action is in the ticket history with actor `agent`.

## Jobs

1. Turn a folder of support mail into a deduplicated, threaded ticket list.
2. Know what is waiting and what is about to breach.
3. Answer consistently (macros) without losing thread headers.
4. Hand bugs to engineering and unhappy customers to account management without copy-paste.

## Today (v0.1.0)

- Ingest `.eml` / maildir with MIME decoding; idempotent; dry run; per-file errors.
- Threading by headers, `[D-n]` tag, then subject + requester within 30 days; own replies recognised by support address.
- Tickets with status, priority, assignee, tags, notes, history.
- Business-hours SLA (AT holidays from craft-biz) with due times and at-risk warning.
- Reply drafts as RFC 5322 `.eml` (CRLF, UTF-8, RFC 2047 subject, In-Reply-To, References), macros with placeholders, mark-as-sent.
- Bus events: `ticket.escalated` -> taskcraft (verified end to end), `signal.negative` -> gaincraft (queued until gaincraft exists).
- 14 MCP tools (docs/TOOLS.md, generated and test-checked), CLI with tables and a conversation view.
- Desktop window (0.2.0): views with counts, ticket table with SLA badges, ticket page (properties, SLA bars, conversation, reply composer with macros, drafts with mark-as-sent, internal notes, escalation, history), status board, unhappy customers, macros, ingest, settings. All 14 tools reachable (test-checked).

## Boundaries

- **No sending.** deskcraft never talks SMTP. Sending stays in the user's mail client until SMTP credentials can come from Windows Credential Manager (craft-biz secrets, not built).
- **No mailbox login.** IMAP only after checking whether lookcraft's local store can be the source, so the fleet has one mail stack, not two.
- **No AI sentiment.** The signal is a documented keyword heuristic. LLM triage may come later through craft-ai-core and will be labelled advisory.
- **No customer portal, no chat, no phone.** E-mail only.
- **Personal data:** mail bodies are stored locally in SQLite. No telemetry, no cloud.

## Gaps (honest)

- The window keeps no composer text between sessions (an unsent reply that was not turned into a draft is lost on close).
- Pending does not pause the SLA clock; SLA targets and business hours are constants, not settings.
- Attachments are not stored (body text only; HTML-only mail is converted to text).
- Subject fallback compares case-insensitively for ASCII only (SQLite `lower`); "Büro" vs "BÜRO" would not match.
- Outlook `.msg` files are not supported; export as `.eml`.
- No merge/split of tickets; no customer/organisation records beyond the e-mail domain.
- Demo data only; never run against a real support mailbox yet.

## Next

1. Attachments: store them on ingest and list them in the conversation.
2. SLA settings (hours, targets) and pause on pending.
3. lookcraft store as an ingest source; then SMTP send behind `--send` with Credential Manager.

## UI/UX Architecture & Menu Bar Standard

In accordance with [`CRAFT_MENU_BAR_STANDARD.md`](../../mcp-central-docs/standards/rust-artcraft/CRAFT_MENU_BAR_STANDARD.md):
- **Pattern 2 (Viewport-Centric Craft Workstation):** Persistent support ticket conversation stage and reply composer viewport with SLA monitor badges. Top quick button row acts as accelerators into the `View` menu. To add a new ticket queue or customer view:
  1. Register a visibility toggle in the `View` or `Window` menu.
  2. Add an entry in the sidebar Ticket Queue / Macro toolbox.
  3. Wire the action directly into the universal `Command::dispatch` bus.
- **Top Menu Bar Hierarchy:**
  - `File`: New Ticket (`Ctrl+N`), Ingest Mailbox / EML (`Ctrl+O`), Open Recent ▸, Save Draft (`Ctrl+S`), Export Thread / EML (`Ctrl+E`), Close Ticket (`Ctrl+W`), Quit (`Ctrl+Q`).
  - `Edit`: Undo (`Ctrl+Z`), Redo (`Ctrl+Y`), Cut, Copy, Paste, Select All, Preferences (`Ctrl+,`).
  - `View`: Ticket Queue List, Active Conversation Thread, Reply Composer, Status Board, SLA Analytics, Theme (`Dark`/`Light`).
  - `Insert`: Add Ticket Note, Insert Macro Template, Attach File, Add Escalation Flag.
  - `Modify`: Update Ticket Status, Reassign Priority, Edit Tags, Update Assignee.
  - `Commands`: Run Ingest Sweep, Calculate SLA Breach Risk, Emit Escalation to TaskCraft, Emit Negative Signal to GainCraft.
  - `Window`: Reset Layout, Ticket Viewport, Conversation History, Drafts Drawer, SLA Watchdog.
  - `Help` (Mandatory 5-Dialog Suite): `Command Palette...` (`Ctrl+K`), `Keyboard Shortcuts...` (`F1`), `Project Files & Docs...`, `Licenses & Notices...`, `About DeskCraft`.
- **Behavioral Mechanics:** Smooth `switch_on_hover` between top-level menus, zero-unsafe immediate-mode rendering, and headless `--smoke` parity.
