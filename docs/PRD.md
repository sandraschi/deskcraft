# deskcraft PRD

## What it is

A local-first support desk for a 1-10 person business: customer e-mails become tickets, replies are drafted with threading intact, service levels are measured in business hours, and problems flow on to engineering (taskcraft) and account health (gaincraft). Pure Rust, ArtCraft pattern: one engine, CLI + MCP today, egui later.

Part of the back-office craft suite: `mcp-central-docs/projects/craft-fleet-expansion/BACKOFFICE_CRAFT_SUITE.md`, section 4.6.

## Users

- The person answering support mail (often the owner). Uses the CLI today, the egui app later.
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

## Boundaries

- **No sending.** deskcraft never talks SMTP. Sending stays in the user's mail client until SMTP credentials can come from Windows Credential Manager (craft-biz secrets, not built).
- **No mailbox login.** IMAP only after checking whether lookcraft's local store can be the source, so the fleet has one mail stack, not two.
- **No AI sentiment.** The signal is a documented keyword heuristic. LLM triage may come later through craft-ai-core and will be labelled advisory.
- **No customer portal, no chat, no phone.** E-mail only.
- **Personal data:** mail bodies are stored locally in SQLite. No telemetry, no cloud.

## Gaps (honest)

- No egui window.
- Pending does not pause the SLA clock; SLA targets and business hours are constants, not settings.
- Attachments are not stored (body text only; HTML-only mail is converted to text).
- Subject fallback compares case-insensitively for ASCII only (SQLite `lower`); "Büro" vs "BÜRO" would not match.
- Outlook `.msg` files are not supported; export as `.eml`.
- No merge/split of tickets; no customer/organisation records beyond the e-mail domain.
- Demo data only; never run against a real support mailbox yet.

## Next

1. egui app: ticket list with SLA colours, conversation view, reply composer with macro picker.
2. SLA settings (hours, targets) and pause on pending.
3. lookcraft store as an ingest source; then SMTP send behind `--send` with Credential Manager.
