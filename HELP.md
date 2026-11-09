# deskcraft help

Task-oriented guide. Every command below also exists as an MCP tool (see [docs/TOOLS.md](docs/TOOLS.md)); add `--json` to any command for machine-readable output. `deskcraft <command> --help` shows all options.

## First setup

```powershell
deskcraft config --address support@yourfirm.at --name "Yourfirm Support" --signature "Yourfirm Support`nWien"
```

The support address matters: mail **from** it is treated as your reply (it stops the first-response clock and is never turned into a ticket).

## Getting mail in

deskcraft reads files; it does not log into a mailbox.

- **Outlook:** dragging mails into Explorer produces `.msg` files, which deskcraft cannot read. Either add the same mailbox to Thunderbird and export from there, or use an Outlook add-in that saves `.eml`.
- **Thunderbird:** ImportExportTools NG > Export all messages in the folder > EML format.
- **Maildir** (from a Linux server or a sync tool): point at the folder that contains `cur/` and `new/`. On Windows, maildir file names use `!` instead of `:`.

```powershell
deskcraft ingest D:\Mail\Support --dry-run   # what would happen
deskcraft ingest D:\Mail\Support             # do it; safe to repeat
```

The result lists new tickets, messages appended to existing tickets, your own replies found, duplicates skipped and files that could not be parsed.

## Daily work

```powershell
deskcraft list                          # new, open, pending; urgent first
deskcraft list --status solved --text drucker
deskcraft show D-12                     # conversation, notes, history, SLA
deskcraft update D-12 --assignee "Anna" --priority high --tags drucker,hardware
deskcraft update D-12 --note "called the customer, toner ordered"
deskcraft update D-12 --status pending  # waiting for the customer
```

Status meanings: **new** (nobody answered), **open** (in progress), **pending** (waiting for the customer), **solved** (answered, may still reopen), **closed** (final; a later mail starts a new ticket).

## Replying

```powershell
deskcraft reply D-12 --body "Hallo {{first_name}}, ..."
deskcraft reply D-12 --macro geloest
```

This writes `outbox\D-12-....eml`. Open it with your mail client (double-click in Explorer opens Outlook or Thunderbird), check it, send it. Then tell deskcraft:

```powershell
deskcraft show D-12          # note the message number (#41 DRAFT)
deskcraft sent 41
```

If you later ingest your Sent folder, deskcraft recognises the reply by its Message-ID and does not store it twice.

Placeholders in bodies and macros: `{{name}}`, `{{first_name}}`, `{{ticket}}`, `{{subject}}`, `{{agent}}`.

## Macros

```powershell
deskcraft macro set geloest "Hallo {{first_name}}, das Problem ist behoben." --status solved --tags solved-macro
deskcraft macro list
```

A macro's status and tags are applied when the draft is written.

## SLA

```powershell
deskcraft sla
deskcraft sla --as-of "2026-11-09 12:00"
```

Business hours Mon-Fri 08:00-17:00, Austrian public holidays excluded. Targets:

| Priority | First response | Resolution |
|---|---|---|
| urgent | 1 h | 8 h |
| high | 4 h | 16 h |
| normal | 8 h | 40 h |
| low | 16 h | 80 h |

`at_risk` means 75 % of the target is used. The clock does not pause while a ticket is pending (known gap).

## Handing off

```powershell
deskcraft escalate D-12 --project OPS --details "reproducible on Chrome"
deskcraft signals            # who is unhappy
deskcraft signals --emit     # tell gaincraft (account health)
```

Escalation goes to taskcraft once per ticket (`taskcraft inbox-drain` turns it into an issue). Signals are keyword based and say so; they are not sentiment AI.

## Troubleshooting

- **"set the support address first"**: run `deskcraft config --address ...`.
- **A reply became a new ticket**: the client dropped In-Reply-To and changed the subject. Merge by hand: add a note and close the duplicate.
- **"file access disabled" in MCP**: start the server with `--allow-path <mail folder>`.
- **Umlauts look wrong in a draft**: the draft is UTF-8; old Outlook versions may need "Encoding > Unicode (UTF-8)" when opening it.
