# AGENTS.md - deskcraft

Support tickets from e-mail. Read first:
`mcp-central-docs/standards/RUST_ARTCRAFT_STANDARD.md`,
`mcp-central-docs/projects/craft-fleet-expansion/BACKOFFICE_CRAFT_SUITE.md` (section 4.6),
then `docs/PRD.md` here.

## Layout

- `crates/deskcraft-store` (L1): tickets, messages, history, macros, emitted events on SQLite WAL.
- `crates/deskcraft-core` (L1): `Command` bus, `dispatch`, MCP catalog, `mail.rs` (parse via mail-parser, subject normalization, `.eml` drafts), `sla.rs` (business hours), ingest and threading, escalation, signals, demo seed.
- `crates/deskcraft-mcp` (L2): `deskcraft` binary. CLI verbs map onto the same tool calls as MCP.
- `crates/deskcraft-app` (L3): desktop window on `craft-biz-ui::app::Shell`. Views change data only via `Frame::run`/`form` (dispatch); reads use the store and `sla_json`. `COVERAGE` maps every tool to its place; a test enforces it.
- Shared kernel: `../craft-biz` (holidays, bus, MCP loop, tools_markdown, confinement).

## Invariants (never weaken)

- deskcraft never sends mail. Replies are `.eml` drafts; `mark_sent` is a user statement.
- Ingest is idempotent: Message-ID unique, raw-bytes SHA-256 unique. Never re-ingest.
- Threading order: headers, `[D-n]` tag, subject + requester within 30 days. Closed tickets never reopen.
- Every ticket change writes history with the actor.
- Events are sent once per ticket (`emitted` table): `ticket.escalated` -> taskcraft, `signal.negative` -> gaincraft.
- The negative signal is a keyword heuristic and must be labelled as such wherever it is shown.

## Docs (keep current in the same commit)

- `README.md`: what it does, quick start, MCP config, build.
- `HELP.md`: task-oriented how-to for users; update when a verb or workflow changes.
- `docs/PRD.md`: purpose, boundaries, honest gaps, next steps; update "Today" and "Gaps" with every release.
- `docs/TOOLS.md`: generated. After any tool change run `just docs`; the `tools_doc_is_current` test fails otherwise.
- `CHANGELOG.md`: one entry per change set.
- CLI help: `about`, per-argument doc comments and `AFTER_HELP` in `main.rs`.

## Rules

- Zero-panic: unwrap/expect/panic/todo/unimplemented denied outside tests.
- MCP mode: stdout is JSON-RPC only; diagnostics to stderr.
- Ingest paths go through `Confinement::check`. The outbox is the app's own folder.
- Demo data only via `demo-seed`, only on an empty desk, labelled DEMO, `.example` domains, mock names.
- Every bug fix ships with a regression test.

## Repo hygiene

- LF line endings only (`.gitattributes`). CRLF breaks commit hooks. (`.eml` drafts are CRLF by RFC; they are data, never committed.)
- Single trunk `main`. No agent branches or leftover WIP branches.
- No em dash characters in code, docs or commit messages.
- No `*.bak`, `*.db`, `*.eml`, `target/`, binaries in git.

## Gates

1. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
2. `cargo test --workspace` (includes the docs/TOOLS.md sync test)
3. `cargo fmt --all --check`
4. MCP probe: `initialize` -> `tools/list` -> `tools/call`, stdout pure JSON
5. `Cargo.lock` committed, clean `git status`
6. `CHANGELOG.md` entry; README / HELP / PRD updated if behaviour changed


## Documentation preservation and hooks

On 2026-11-20 a README "standardization" sweep across the fleet deleted project descriptions,
status sections and user warnings in nine repos. Full rules:
`mcp-central-docs/standards/CONTENT_PRESERVATION_STANDARD.md`.

- Applying any standard or template restructures; it never deletes. Move content (within the file
  or to `docs/` with a link), do not drop it. Never remove what the project is, what it is based on,
  its status (what works, what does not, what is not built) or warnings to users.
- Remove content only when Sandra explicitly asks for that removal, in this repo.
- Never add claims you have not verified (release downloads, versions, test counts, features).
- Only standards with `status: active` may be applied across repos. A change across several repos is
  done on one repo first and approved by Sandra; its commits name the tool and carry a `Sweep-Id:` line.
- `.githooks/commit-msg` blocks a commit that deletes a Markdown file, removes 20% of its lines
  (15 or more) or removes 3 or more headings. A removal Sandra asked for passes with
  `Docs-Removal-Approved: <who and why>` in the message. Enable once per clone:
  `git config core.hooksPath .githooks`. Never use `--no-verify`; if a hook blocks you, stop and ask.
