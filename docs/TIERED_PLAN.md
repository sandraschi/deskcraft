# DeskCraft - Tiered Execution Plan

**Parent Standard:** [`RUST_ARTCRAFT_STANDARD.md`](../../mcp-central-docs/standards/RUST_ARTCRAFT_STANDARD.md)  
**Menu Bar Standard:** [`CRAFT_MENU_BAR_STANDARD.md`](../../mcp-central-docs/standards/rust-artcraft/CRAFT_MENU_BAR_STANDARD.md)  
**Architecture Model:** 4-Tier ArtCraft Incremental Delivery  
**Status:** Active  

---

## Architecture Flow

```
Tier 1: Core Engine ──> Tier 2: MCP / CLI ──> Tier 3: Desktop UI ──> Tier 4: Menu Bar, Help & Packaging
(deskcraft-core)       (deskcraft-mcp)       (deskcraft-app)       (Hardened Workstation)
```

---

## Tier Specifications

### Tier 1 - Core Support & SLA Engine (`deskcraft-core`)
- [x] L1 `Command::dispatch` universal support execution bus.
- [x] MIME email ingest (.eml, maildir) with RFC 5322 parsing.
- [x] Header-based conversation threading and requester deduplication.
- [x] Business-hours SLA calculator with holiday awareness.
- [x] Reply composer with RFC 2047 formatting and macro template expansion.
- [x] Cross-craft bus event emitter (`ticket.escalated` -> TaskCraft, `signal.negative` -> GainCraft).
- [x] `#![forbid(unsafe_code)]` and zero-panic clippy lints.

### Tier 2 - Headless Driver & FastMCP (`deskcraft-mcp`)
- [x] Unified CLI `deskcraft-cli` with `--json` output.
- [x] FastMCP stdio server communicating over JSON-RPC 2.0 (14 tools).
- [x] Conversation export and thread history inspection.
- [x] Headless `--smoke` verification without window initialization.

### Tier 3 - Desktop Workstation UI (`deskcraft-app`)
- [x] Immediate-mode `egui` 0.37 / `wgpu` workstation with 14 tool entry points.
- [x] Ticket table with SLA countdown badges and priority highlights.
- [x] Central conversation thread view with reply composer and macro picker.
- [x] Status Kanban board and unhappy customer monitor.

### Tier 4 - Craft Menu Bar Standard, Help Suite & Hardened Verification
- [ ] Universal 8-menu bar (`File`, `Edit`, `View`, `Insert`, `Modify`, `Commands`, `Window`, `Help`) with `switch_on_hover`.
- [ ] Mandatory 5-dialog Help suite (`Command Palette...`, `Keyboard Shortcuts...`, `Project Files & Docs...`, `Licenses & Notices...`, `About DeskCraft`).
- [ ] Fleet UI/UX Pattern 2 adherence: viewport-centric navigation, top button row mapped to View toggles, panel visibility managed via `View` / `Window`.
- [ ] Tofu Guard verification (100% font glyph coverage, zero tofu placeholder boxes).
- [ ] Headless `--smoke` parity validation.
- [ ] Standalone release binary packaging and NSIS installer.
