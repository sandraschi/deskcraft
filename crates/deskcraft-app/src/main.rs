//! DeskCraft desktop window, laid out like a help desk: views sidebar,
//! ticket table with SLA badges, ticket page (properties left, conversation
//! centre, reply composer with macros), status board, unhappy customers,
//! macros, mail ingest, settings.
//!
//! Every change goes through `deskcraft_core::dispatch` (same path as CLI and
//! MCP). Reads use the store and `sla_json` (no side effects).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Local, NaiveDateTime};
use craft_biz::bus::Bus;
use craft_biz_ui::app::{self, Frame, NavEntry, SuiteApp, Wiring};
use craft_biz_ui::kanban::{self, Board, Card, Column, Lane};
use craft_biz_ui::table::{Cell, Col, Table, TableEvent};
use craft_biz_ui::theme::{self, semibold};
use craft_biz_ui::widgets::{self, Kpi};
use craft_biz_ui::Tone;
use deskcraft_core::{dispatch, sla_json, tools, Command, Confinement, Ctx};
use deskcraft_store::{Desk, HistoryRow, Macro, Message, Priority, Status, Ticket};
use egui::{RichText, Ui};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const V_UNSOLVED: usize = 0;
const V_UNASSIGNED: usize = 1;
const V_URGENT: usize = 2;
const V_SLA: usize = 3;
const V_PENDING: usize = 4;
const V_SOLVED: usize = 5;
const V_BOARD: usize = 6;
const V_UNHAPPY: usize = 7;
const V_MACROS: usize = 8;
const V_INGEST: usize = 9;
const V_SETTINGS: usize = 10;

/// Every MCP tool and where the window performs it (checked by a test).
pub const COVERAGE: &[(&str, &str)] = &[
    ("desk_status", "Nav counts; Settings > Status"),
    (
        "desk_config",
        "Settings > Support address, sender name, signature",
    ),
    ("desk_ingest", "Ingest mail > Dry run, Ingest"),
    (
        "desk_ticket_list",
        "Views: Unsolved, Unassigned, Urgent, SLA, Pending, Solved",
    ),
    ("desk_ticket_show", "Ticket page > Full record"),
    (
        "desk_ticket_update",
        "Ticket page properties and internal note; Board drag",
    ),
    (
        "desk_reply_draft",
        "Ticket page > Reply composer (with macro)",
    ),
    ("desk_reply_mark_sent", "Ticket page > draft > Mark as sent"),
    ("desk_macro_set", "Macros > New macro, Edit"),
    ("desk_macro_list", "Macros view; composer macro picker"),
    ("desk_sla_status", "SLA view; SLA badges everywhere"),
    ("desk_escalate", "Ticket page > Escalate to taskcraft"),
    ("desk_signals", "Unhappy customers > Send to gaincraft"),
    ("desk_demo_seed", "Empty state; Settings > Demo data"),
];

#[derive(Clone)]
struct Sla {
    fr: String,
    fr_due: String,
    res: String,
    res_due: String,
}

struct Row {
    t: Ticket,
    sla: Sla,
}

struct Open {
    key: String,
    messages: Vec<Message>,
    history: Vec<HistoryRow>,
    escalated: Option<String>,
}

pub struct DeskApp {
    ctx: Ctx,
    db: String,
    now: NaiveDateTime,
    rows: Vec<Row>,
    solved: Vec<Ticket>,
    macros: Vec<Macro>,
    unhappy: Vec<Value>,
    config: Value,
    open: Option<Open>,
    last_view: usize,
    reply: String,
    reply_macro: String,
    note: String,
    ingest_path: String,
    board_lanes: bool,
}

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

fn prio_tone(p: Priority) -> Tone {
    match p {
        Priority::Urgent => Tone::Danger,
        Priority::High => Tone::Warn,
        Priority::Normal => Tone::Accent,
        Priority::Low => Tone::Neutral,
    }
}

fn status_tone(s: Status) -> Tone {
    match s {
        Status::New => Tone::Warn,
        Status::Open => Tone::Danger,
        Status::Pending => Tone::Info,
        Status::Solved => Tone::Ok,
        Status::Closed => Tone::Neutral,
    }
}

fn status_label(s: Status, de: bool) -> &'static str {
    match (s, de) {
        (Status::New, true) => "Neu",
        (Status::New, false) => "New",
        (Status::Open, true) => "Offen",
        (Status::Open, false) => "Open",
        (Status::Pending, true) => "Wartend",
        (Status::Pending, false) => "Pending",
        (Status::Solved, true) => "Gelöst",
        (Status::Solved, false) => "Solved",
        (Status::Closed, true) => "Geschlossen",
        (Status::Closed, false) => "Closed",
    }
}

fn prio_label(p: Priority, de: bool) -> &'static str {
    match (p, de) {
        (Priority::Urgent, true) => "Dringend",
        (Priority::Urgent, false) => "Urgent",
        (Priority::High, true) => "Hoch",
        (Priority::High, false) => "High",
        (Priority::Normal, _) => "Normal",
        (Priority::Low, true) => "Niedrig",
        (Priority::Low, false) => "Low",
    }
}

fn sla_tone(state: &str) -> Tone {
    match state {
        "breached" => Tone::Danger,
        "at_risk" => Tone::Warn,
        "met" => Tone::Ok,
        _ => Tone::Neutral,
    }
}

fn sla_text(state: &str, due: &str, de: bool) -> String {
    let hhmm = due.get(11..16).unwrap_or("");
    let day = due
        .get(8..10)
        .zip(due.get(5..7))
        .map(|(d, m)| format!("{d}.{m}."))
        .unwrap_or_default();
    match (state, de) {
        ("breached", true) => "überschritten".into(),
        ("breached", false) => "breached".into(),
        ("met", true) => "erfüllt".into(),
        ("met", false) => "met".into(),
        ("at_risk", true) => format!("knapp, {day} {hhmm}"),
        ("at_risk", false) => format!("at risk, {day} {hhmm}"),
        (_, true) => format!("bis {day} {hhmm}"),
        (_, false) => format!("due {day} {hhmm}"),
    }
}

fn worst_sla(s: &Sla) -> (&str, &str) {
    let rank = |x: &str| match x {
        "breached" => 3,
        "at_risk" => 2,
        "ok" => 1,
        _ => 0,
    };
    if s.fr != "met" && rank(&s.fr) >= rank(&s.res) {
        (s.fr.as_str(), s.fr_due.as_str())
    } else {
        (s.res.as_str(), s.res_due.as_str())
    }
}

fn dt(s: &NaiveDateTime) -> String {
    s.format("%d.%m.%Y %H:%M").to_string()
}

impl DeskApp {
    pub fn new(ctx: Ctx, db: String) -> DeskApp {
        DeskApp {
            ctx,
            db,
            now: now(),
            rows: Vec::new(),
            solved: Vec::new(),
            macros: Vec::new(),
            unhappy: Vec::new(),
            config: Value::Null,
            open: None,
            last_view: 0,
            reply: String::new(),
            reply_macro: String::new(),
            note: String::new(),
            ingest_path: String::new(),
            board_lanes: false,
        }
    }

    fn open_ticket(&mut self, key: &str) {
        let d = &self.ctx.desk;
        let r = (|| -> Result<Open, String> {
            let t = d.ticket(key).map_err(|e| e.to_string())?;
            Ok(Open {
                key: t.key.clone(),
                messages: d.messages(t.id).map_err(|e| e.to_string())?,
                history: d.history(t.id).map_err(|e| e.to_string())?,
                escalated: d
                    .emitted(t.id, "ticket.escalated")
                    .map_err(|e| e.to_string())?,
            })
        })();
        match r {
            Ok(o) => {
                if self.open.as_ref().map(|x| &x.key) != Some(&o.key) {
                    self.reply.clear();
                    self.note.clear();
                    self.reply_macro.clear();
                }
                self.open = Some(o);
            }
            Err(e) => eprintln!("deskcraft-app: {e}"),
        }
    }

    fn ticket(&self, key: &str) -> Option<&Ticket> {
        self.rows
            .iter()
            .map(|r| &r.t)
            .chain(self.solved.iter())
            .find(|t| t.key == key)
    }

    fn list_for(&self, view: usize) -> Vec<&Row> {
        self.rows
            .iter()
            .filter(|r| match view {
                V_UNASSIGNED => r.t.assignee.is_none(),
                V_URGENT => matches!(r.t.priority, Priority::Urgent | Priority::High),
                V_SLA => {
                    let (s, _) = worst_sla(&r.sla);
                    s == "breached" || s == "at_risk"
                }
                V_PENDING => r.t.status == Status::Pending,
                _ => true,
            })
            .collect()
    }

    fn assignees(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .rows
            .iter()
            .filter_map(|r| r.t.assignee.clone())
            .chain(self.solved.iter().filter_map(|t| t.assignee.clone()))
            .collect();
        v.push(self.ctx.actor.clone());
        v.sort();
        v.dedup();
        v
    }

    fn empty(&mut self, ui: &mut Ui, f: &mut Frame) {
        widgets::card(ui, |ui| {
            widgets::empty_state(
                ui,
                f.t("Keine Tickets", "No tickets"),
                f.t(
                    "Mails einlesen (Ordner mit .eml oder Maildir) oder Demo-Daten laden.",
                    "Ingest mail (a folder of .eml files or a maildir) or load the demo data.",
                ),
            );
            ui.vertical_centered(|ui| {
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 330.0).max(0.0) / 2.0);
                    if widgets::primary(ui, f.t("Mails einlesen", "Ingest mail")).clicked() {
                        f.goto(V_INGEST);
                    }
                    if ui
                        .button(f.t("Demo-Daten laden", "Load demo data"))
                        .clicked()
                    {
                        f.run(f.t("Demo-Daten", "Demo data"), "desk_demo_seed", json!({}));
                    }
                });
            });
        });
    }

    // ---------------- ticket list ----------------

    fn view_list(&mut self, ui: &mut Ui, view: usize, f: &mut Frame) {
        let de = f.lang_de;
        let (title, sub) = match view {
            V_UNASSIGNED => (f.t("Nicht zugewiesen", "Unassigned"), f.t("Offene Tickets ohne Bearbeitung.", "Unsolved tickets nobody owns.")),
            V_URGENT => (f.t("Dringend und hoch", "Urgent and high"), f.t("Nach Priorität, dann nach Alter.", "By priority, then by age.")),
            V_SLA => (f.t("SLA in Gefahr", "SLA at risk"), f.t("Erste Antwort oder Lösung knapp oder überschritten (Geschäftszeit Mo-Fr 08-17, AT-Feiertage frei).", "First response or resolution at risk or breached (business hours Mon-Fri 08-17, AT holidays off).")),
            V_PENDING => (f.t("Wartend", "Pending"), f.t("Wartet auf den Kunden.", "Waiting for the customer.")),
            V_SOLVED => (f.t("Gelöst", "Recently solved"), ""),
            _ => (f.t("Alle offenen Tickets", "All unsolved tickets"), f.t("Klick öffnet das Ticket.", "Click a ticket to open it.")),
        };
        widgets::page_header(ui, title, sub, |ui| {
            if ui
                .button(f.t("SLA-Bericht", "SLA report"))
                .on_hover_text("desk_sla_status")
                .clicked()
            {
                f.run_show("desk_sla_status", json!({}));
            }
        });
        if self.rows.is_empty() && self.solved.is_empty() {
            self.empty(ui, f);
            return;
        }
        if view == V_UNSOLVED {
            let breached = self
                .rows
                .iter()
                .filter(|r| worst_sla(&r.sla).0 == "breached")
                .count();
            let risk = self
                .rows
                .iter()
                .filter(|r| worst_sla(&r.sla).0 == "at_risk")
                .count();
            let unassigned = self.rows.iter().filter(|r| r.t.assignee.is_none()).count();
            let urgent = self
                .rows
                .iter()
                .filter(|r| r.t.priority == Priority::Urgent)
                .count();
            widgets::kpi_row(
                ui,
                &[
                    Kpi::new(f.t("Offen", "Unsolved"), self.rows.len().to_string()),
                    Kpi::new(
                        f.t("Nicht zugewiesen", "Unassigned"),
                        unassigned.to_string(),
                    )
                    .note(
                        if unassigned > 0 {
                            f.t("übernehmen", "pick up")
                        } else {
                            "ok"
                        },
                        if unassigned > 0 { Tone::Warn } else { Tone::Ok },
                    ),
                    Kpi::new(f.t("Dringend", "Urgent"), urgent.to_string()),
                    Kpi::new("SLA", format!("{breached} / {risk}")).note(
                        f.t("überschritten / knapp", "breached / at risk"),
                        if breached > 0 {
                            Tone::Danger
                        } else if risk > 0 {
                            Tone::Warn
                        } else {
                            Tone::Ok
                        },
                    ),
                ],
            );
            ui.add_space(6.0);
        }
        let cols = vec![
            Col::new("", 0.35).min(26.0),
            Col::new("ID", 0.6).min(54.0),
            Col::new(f.t("Betreff", "Subject"), 3.0).min(180.0),
            Col::new(f.t("Anfragende", "Requester"), 1.8).prio(2),
            Col::new(f.t("Priorität", "Priority"), 0.9),
            Col::new("Status", 0.9),
            Col::new("SLA", 1.3),
            Col::new(f.t("Bearbeitung", "Assignee"), 1.4).prio(1),
            Col::new("Tags", 1.4).prio(3),
            Col::new(f.t("Eingang", "Opened"), 1.1).min(128.0).prio(4),
        ];
        let mk = |t: &Ticket, sla: Option<&Sla>| -> (String, Vec<Cell>) {
            let (state, due) = sla.map(worst_sla).unwrap_or(("met", ""));
            (
                t.key.clone(),
                vec![
                    Cell::Dot(status_tone(t.status)),
                    Cell::Mono(t.key.clone()),
                    Cell::Strong(t.subject.clone()),
                    Cell::Text(
                        t.requester_name
                            .clone()
                            .unwrap_or_else(|| t.requester_email.clone()),
                    ),
                    Cell::Pill(prio_label(t.priority, de).into(), prio_tone(t.priority)),
                    Cell::Pill(status_label(t.status, de).into(), status_tone(t.status)),
                    if sla.is_some() {
                        Cell::Pill(sla_text(state, due, de), sla_tone(state))
                    } else {
                        Cell::Weak("-".into())
                    },
                    t.assignee
                        .clone()
                        .map(Cell::Person)
                        .unwrap_or(Cell::Weak(f.t("niemand", "nobody").into())),
                    Cell::Pills(t.tags.iter().map(|x| (x.clone(), Tone::Neutral)).collect()),
                    Cell::Weak(dt(&t.opened_at)),
                ],
            )
        };
        let data: Vec<(String, Vec<Cell>)> = if view == V_SOLVED {
            self.solved
                .iter()
                .filter(|t| app::search_hit(&f.search, &[&t.subject, &t.key, &t.requester_email]))
                .map(|t| mk(t, None))
                .collect()
        } else {
            let mut list = self.list_for(view);
            if view == V_URGENT {
                list.sort_by_key(|r| (std::cmp::Reverse(r.t.priority as u8), r.t.opened_at));
            }
            list.into_iter()
                .filter(|r| {
                    app::search_hit(
                        &f.search,
                        &[
                            &r.t.subject,
                            &r.t.key,
                            &r.t.requester_email,
                            r.t.assignee.as_deref().unwrap_or(""),
                        ],
                    )
                })
                .map(|r| mk(&r.t, Some(&r.sla)))
                .collect()
        };
        let mut pick = None;
        widgets::card(ui, |ui| {
            if data.is_empty() {
                widgets::empty_state(
                    ui,
                    f.t("Nichts zu tun", "Nothing here"),
                    f.t(
                        "Keine Tickets in dieser Ansicht.",
                        "No tickets in this view.",
                    ),
                );
            }
            for e in Table::new(("desk_list", view), &cols, &data).show(ui) {
                let (TableEvent::Select(k) | TableEvent::Open(k)) = e;
                pick = Some(k);
            }
        });
        if let Some(k) = pick {
            self.open_ticket(&k);
        }
    }

    // ---------------- ticket page ----------------

    fn view_ticket(&mut self, ui: &mut Ui, f: &mut Frame) {
        let de = f.lang_de;
        let Some(o) = &self.open else { return };
        let Some(t) = self.ticket(&o.key).cloned() else {
            if ui.button(f.t("< Zurück", "< Back")).clicked() {
                self.open = None;
            }
            ui.label(f.t("Ticket nicht gefunden.", "Ticket not found."));
            return;
        };
        let key = t.key.clone();
        let sla = sla_json(&t, self.now);
        let tk = theme::current(ui.ctx());
        let mut back = false;
        ui.horizontal(|ui| {
            if ui
                .button(f.t("< Zurück zur Liste", "< Back to list"))
                .clicked()
            {
                back = true;
            }
            ui.label(
                RichText::new(&key)
                    .family(egui::FontFamily::Monospace)
                    .size(15.0),
            );
            ui.label(RichText::new(&t.subject).family(semibold()).size(19.0));
        });
        if back {
            self.open = None;
            return;
        }
        ui.add_space(6.0);
        egui::Panel::left(("desk_props", &key))
            .resizable(false)
            .exact_size(270.0)
            .frame(
                egui::Frame::new()
                    .fill(tk.surface)
                    .stroke(egui::Stroke::new(1.0, tk.border))
                    .corner_radius(10)
                    .inner_margin(12),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("props_scroll")
                    .show(ui, |ui| {
                        widgets::section(ui, f.t("Anfragende", "Requester"), None);
                        ui.horizontal(|ui| {
                            let who = t
                                .requester_name
                                .clone()
                                .unwrap_or_else(|| t.requester_email.clone());
                            widgets::avatar(ui, &who, 12.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&who).family(semibold()));
                                ui.label(
                                    RichText::new(&t.requester_email)
                                        .color(tk.text_weak)
                                        .size(13.0),
                                );
                            });
                        });
                        ui.add_space(8.0);
                        ui.label(RichText::new("Status").family(semibold()));
                        let mut st = t.status;
                        egui::ComboBox::from_id_salt("st")
                            .selected_text(status_label(st, de))
                            .width(230.0)
                            .show_ui(ui, |ui| {
                                for s in Status::ALL {
                                    ui.selectable_value(&mut st, *s, status_label(*s, de));
                                }
                            });
                        if st != t.status {
                            f.run(
                                f.t("Status geändert", "Status changed"),
                                "desk_ticket_update",
                                json!({ "ticket": key, "status": st.as_str() }),
                            );
                        }
                        ui.label(RichText::new(f.t("Priorität", "Priority")).family(semibold()));
                        let mut pr = t.priority;
                        egui::ComboBox::from_id_salt("pr")
                            .selected_text(prio_label(pr, de))
                            .width(230.0)
                            .show_ui(ui, |ui| {
                                for p in Priority::ALL {
                                    ui.selectable_value(&mut pr, *p, prio_label(*p, de));
                                }
                            });
                        if pr != t.priority {
                            f.run(
                                f.t("Priorität geändert", "Priority changed"),
                                "desk_ticket_update",
                                json!({ "ticket": key, "priority": pr.as_str() }),
                            );
                        }
                        ui.label(RichText::new(f.t("Bearbeitung", "Assignee")).family(semibold()));
                        let mut asg = t.assignee.clone().unwrap_or_default();
                        egui::ComboBox::from_id_salt("as")
                            .selected_text(if asg.is_empty() {
                                f.t("niemand", "nobody").to_string()
                            } else {
                                asg.clone()
                            })
                            .width(230.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut asg,
                                    String::new(),
                                    f.t("niemand", "nobody"),
                                );
                                for a in self.assignees() {
                                    ui.selectable_value(&mut asg, a.clone(), a);
                                }
                            });
                        if asg != t.assignee.clone().unwrap_or_default() {
                            let v = if asg.is_empty() {
                                Value::Null
                            } else {
                                Value::String(asg.clone())
                            };
                            f.run(
                                f.t("Zugewiesen", "Assigned"),
                                "desk_ticket_update",
                                json!({ "ticket": key, "assignee": v }),
                            );
                        }
                        if t.assignee.as_deref() != Some(self.ctx.actor.as_str())
                            && ui.button(f.t("Mir zuweisen", "Take it")).clicked()
                        {
                            f.run(
                                f.t("Zugewiesen", "Assigned"),
                                "desk_ticket_update",
                                json!({ "ticket": key, "assignee": self.ctx.actor }),
                            );
                        }
                        ui.label(RichText::new("Tags").family(semibold()));
                        ui.horizontal_wrapped(|ui| {
                            for tag in &t.tags {
                                widgets::pill(ui, tag, Tone::Neutral);
                            }
                            if ui.small_button(f.t("ändern", "edit")).clicked() {
                                f.form(
                                    "desk_ticket_update",
                                    &[("ticket", &key), ("tags", &t.tags.join(","))],
                                );
                            }
                        });
                        ui.add_space(8.0);
                        widgets::section(ui, "SLA", None);
                        for (label, part) in [
                            (f.t("Erste Antwort", "First response"), "first_response"),
                            (f.t("Lösung", "Resolution"), "resolution"),
                        ] {
                            let state = sla[part]["state"].as_str().unwrap_or("");
                            let due = sla[part]["due"].as_str().unwrap_or("");
                            ui.horizontal(|ui| {
                                ui.label(label);
                                widgets::pill(ui, &sla_text(state, due, de), sla_tone(state));
                            });
                            let used = sla[part]["used_minutes"].as_i64().unwrap_or(0);
                            let target = sla[part]["target_minutes"].as_i64().unwrap_or(1).max(1);
                            widgets::meter(ui, used as f32 / target as f32, 230.0, sla_tone(state))
                                .on_hover_text(format!("{used} / {target} min"));
                        }
                        ui.add_space(8.0);
                        widgets::section(ui, f.t("Weitergeben", "Hand over"), None);
                        match &o.escalated {
                            Some(ev) => {
                                widgets::pill(
                                    ui,
                                    f.t("an taskcraft gesendet", "sent to taskcraft"),
                                    Tone::Ok,
                                )
                                .on_hover_text(format!("event {ev}"));
                            }
                            None => {
                                if ui
                                    .button(f.t(
                                        "An taskcraft eskalieren...",
                                        "Escalate to taskcraft...",
                                    ))
                                    .on_hover_text(
                                        "becomes an issue in taskcraft (once per ticket)",
                                    )
                                    .clicked()
                                {
                                    f.form("desk_escalate", &[("ticket", &key)]);
                                }
                            }
                        }
                        ui.add_space(8.0);
                        if ui.button(f.t("Ganzer Datensatz", "Full record")).clicked() {
                            f.run_show("desk_ticket_show", json!({ "ticket": key }));
                        }
                        ui.add_space(8.0);
                        egui::CollapsingHeader::new(
                            RichText::new(format!(
                                "{} ({})",
                                f.t("Verlauf", "History"),
                                o.history.len()
                            ))
                            .family(semibold()),
                        )
                        .show(ui, |ui| {
                            for h in o.history.iter().rev() {
                                ui.label(
                                    RichText::new(format!(
                                        "{}  {}",
                                        h.at.get(..16).unwrap_or(&h.at).replace('T', " "),
                                        h.actor
                                    ))
                                    .size(12.5)
                                    .color(tk.text_weak),
                                );
                                ui.label(format!(
                                    "{}: {} -> {}",
                                    h.field,
                                    h.old.clone().unwrap_or_else(|| "-".into()),
                                    h.new.clone().unwrap_or_else(|| "-".into())
                                ));
                            }
                        });
                    });
            });

        // Conversation and composer.
        let mut send_draft = false;
        let mut add_note = false;
        egui::ScrollArea::vertical().id_salt(("conv", &key)).auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(2.0);
            for m in &o.messages {
                message_bubble(ui, m, de, f);
                ui.add_space(8.0);
            }
            ui.add_space(6.0);
            widgets::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(f.t("Antwort", "Reply")).family(semibold()).size(16.0));
                    ui.label(RichText::new(format!("{} {}", f.t("an", "to"), t.requester_email)).color(tk.text_weak));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        egui::ComboBox::from_id_salt("macro").selected_text(if self.reply_macro.is_empty() { f.t("Makro...", "Macro...").to_string() } else { self.reply_macro.clone() }).width(200.0).show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.reply_macro, String::new(), f.t("(kein Makro)", "(no macro)"));
                            for m in &self.macros {
                                ui.selectable_value(&mut self.reply_macro, m.name.clone(), &m.name).on_hover_text(&m.body);
                            }
                        });
                    });
                });
                if let Some(m) = self.macros.iter().find(|m| m.name == self.reply_macro) {
                    let mut info = m.body.chars().take(200).collect::<String>();
                    if let Some(s) = m.set_status {
                        info.push_str(&format!("\n-> {} {}", f.t("Status", "status"), status_label(s, de)));
                    }
                    ui.label(RichText::new(info).color(tk.text_weak).size(13.0));
                }
                ui.add(egui::TextEdit::multiline(&mut self.reply).desired_rows(6).desired_width(f32::INFINITY).hint_text(f.t("Antwort schreiben... (mit Makro: Text wird vor den Makrotext gestellt)", "Write a reply... (with a macro: your text goes before the macro text)")));
                ui.horizontal(|ui| {
                    let ok = !self.reply.trim().is_empty() || !self.reply_macro.is_empty();
                    if ui.add_enabled(ok, egui::Button::new(RichText::new(f.t("Entwurf erstellen (.eml)", "Create draft (.eml)")).family(semibold()).color(tk.on_accent)).fill(tk.accent)).clicked() {
                        send_draft = true;
                    }
                    ui.label(RichText::new(f.t("deskcraft sendet nie selbst: der Entwurf landet im Postausgang-Ordner, gesendet wird aus dem Mailprogramm.", "deskcraft never sends: the draft goes to the outbox folder, you send it from your mail client.")).color(tk.text_weak).size(12.5));
                });
            });
            ui.add_space(8.0);
            widgets::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(f.t("Interne Notiz", "Internal note")).family(semibold()));
                ui.add(egui::TextEdit::multiline(&mut self.note).desired_rows(2).desired_width(f32::INFINITY).hint_text(f.t("Nur für das Team sichtbar", "Visible to the team only")));
                if ui.add_enabled(!self.note.trim().is_empty(), egui::Button::new(f.t("Notiz speichern", "Save note"))).clicked() {
                    add_note = true;
                }
            });
        });
        if send_draft {
            let mut args = json!({ "ticket": key });
            if !self.reply.trim().is_empty() {
                args["body"] = json!(self.reply.trim());
            }
            if !self.reply_macro.is_empty() {
                args["macro"] = json!(self.reply_macro);
            }
            f.run(
                f.t("Entwurf im Postausgang", "Draft in the outbox"),
                "desk_reply_draft",
                args,
            );
            self.reply.clear();
            self.reply_macro.clear();
        }
        if add_note {
            f.run(
                f.t("Notiz gespeichert", "Note saved"),
                "desk_ticket_update",
                json!({ "ticket": key, "note": self.note.trim() }),
            );
            self.note.clear();
        }
    }

    // ---------------- board ----------------

    fn view_board(&mut self, ui: &mut Ui, f: &mut Frame) {
        let de = f.lang_de;
        let mut lanes = self.board_lanes;
        widgets::page_header(
            ui,
            f.t("Ticket-Board", "Ticket board"),
            f.t(
                "Karten ziehen ändert den Status. Doppelklick öffnet das Ticket.",
                "Drag a card to change the status. Double-click opens the ticket.",
            ),
            |ui| {
                ui.checkbox(
                    &mut lanes,
                    f.t("Bahnen nach Priorität", "Swimlanes by priority"),
                );
            },
        );
        self.board_lanes = lanes;
        if self.rows.is_empty() && self.solved.is_empty() {
            self.empty(ui, f);
            return;
        }
        let columns = vec![
            Column::new("new", status_label(Status::New, de), Tone::Warn),
            Column::new("open", status_label(Status::Open, de), Tone::Danger),
            Column::new("pending", status_label(Status::Pending, de), Tone::Info),
            Column::new("solved", status_label(Status::Solved, de), Tone::Ok),
        ];
        let lane_list: Vec<Lane> = if self.board_lanes {
            Priority::ALL
                .iter()
                .rev()
                .map(|p| Lane {
                    id: p.as_str().into(),
                    title: prio_label(*p, de).into(),
                })
                .collect()
        } else {
            Vec::new()
        };
        let mk = |t: &Ticket, sla: Option<&Sla>| -> Card {
            let mut meta = vec![t
                .requester_name
                .clone()
                .unwrap_or_else(|| t.requester_email.clone())];
            let badge = sla.map(|s| {
                let (st, due) = worst_sla(s);
                (sla_text(st, due, de), sla_tone(st))
            });
            if t.message_count > 0 {
                meta.push(format!("{} {}", t.message_count, f.t("Mails", "mails")));
            }
            Card {
                id: t.key.clone(),
                column: t.status.as_str().into(),
                lane: if self.board_lanes {
                    t.priority.as_str().into()
                } else {
                    String::new()
                },
                key: t.key.clone(),
                title: t.subject.clone(),
                tags: std::iter::once((
                    prio_label(t.priority, de).to_string(),
                    prio_tone(t.priority),
                ))
                .chain(t.tags.iter().take(3).map(|x| (x.clone(), Tone::Neutral)))
                .collect(),
                accent: prio_tone(t.priority),
                badge,
                assignee: t.assignee.clone(),
                meta,
                blocked: None,
                done: t.status == Status::Solved,
            }
        };
        let cards: Vec<Card> = self
            .rows
            .iter()
            .map(|r| mk(&r.t, Some(&r.sla)))
            .chain(
                self.solved
                    .iter()
                    .filter(|t| t.status == Status::Solved)
                    .take(30)
                    .map(|t| mk(t, None)),
            )
            .filter(|c| app::search_hit(&f.search, &[&c.title, &c.key, &c.meta.join(" ")]))
            .collect();
        let msg_new = f
            .t(
                "Neu setzt nur eine eingehende Mail.",
                "Only an incoming mail makes a ticket new.",
            )
            .to_string();
        let rule = move |_c: &Card, to: &str| -> Result<(), String> {
            if to == "new" {
                Err(msg_new.clone())
            } else {
                Ok(())
            }
        };
        let events = {
            let r: kanban::Rule = &rule;
            let mut b = Board::new("desk_board", &columns, &cards)
                .rule(r)
                .add_buttons(false)
                .selected(self.open.as_ref().map(|o| o.key.as_str()));
            if self.board_lanes {
                b = b.lanes(&lane_list);
            }
            b.show(ui)
        };
        for e in events {
            match e {
                kanban::Event::Moved {
                    card,
                    to,
                    from,
                    to_lane,
                    from_lane,
                    ..
                } => {
                    let mut args = json!({ "ticket": card });
                    if to != from {
                        args["status"] = json!(to);
                    }
                    if to_lane != from_lane && !to_lane.is_empty() {
                        args["priority"] = json!(to_lane);
                    }
                    if args.as_object().is_some_and(|o| o.len() > 1) {
                        f.run(
                            f.t("Ticket verschoben", "Ticket moved"),
                            "desk_ticket_update",
                            args,
                        );
                    }
                }
                kanban::Event::Refused { reason, .. } => f.toast(reason, Tone::Warn),
                kanban::Event::Open(k) => {
                    self.open_ticket(&k);
                    f.goto(V_UNSOLVED);
                    self.last_view = V_UNSOLVED;
                }
                kanban::Event::Select(_) | kanban::Event::Add { .. } | kanban::Event::Undo => {}
            }
        }
    }

    // ---------------- other views ----------------

    fn view_unhappy(&mut self, ui: &mut Ui, f: &mut Frame) {
        let pending = self
            .unhappy
            .iter()
            .filter(|r| r["sent_to_gaincraft"].is_null())
            .count();
        widgets::page_header(ui, f.t("Unzufriedene Kunden", "Unhappy customers"), f.t("Deterministische Stichwortsuche (DE/EN), keine KI. Senden schickt jedes Ticket einmal an gaincraft (signal.negative).", "Deterministic keyword match (DE/EN), not AI. Sending passes each ticket once to gaincraft (signal.negative)."), |ui| {
            if ui.add_enabled(pending > 0, egui::Button::new(RichText::new(format!("{} ({pending})", f.t("An gaincraft senden", "Send to gaincraft"))).family(semibold()))).clicked() {
                f.run_show("desk_signals", json!({ "emit": true }));
            }
        });
        if self.unhappy.is_empty() {
            widgets::card(ui, |ui| {
                widgets::empty_state(
                    ui,
                    f.t("Keine unzufriedenen Mails", "No unhappy mails"),
                    f.t("Gut so.", "Good."),
                )
            });
            return;
        }
        let cols = vec![
            Col::new("ID", 0.6),
            Col::num("Score", 0.6),
            Col::new(f.t("Betreff", "Subject"), 2.6),
            Col::new(f.t("Anfragende", "Requester"), 1.8).prio(1),
            Col::new(f.t("Treffer", "Hits"), 2.2).prio(2),
            Col::new("gaincraft", 1.0),
        ];
        let data: Vec<(String, Vec<Cell>)> = self
            .unhappy
            .iter()
            .map(|r| {
                let score = r["score"].as_i64().unwrap_or(0);
                let sent = !r["sent_to_gaincraft"].is_null();
                (
                    r["ticket"].as_str().unwrap_or("").to_string(),
                    vec![
                        Cell::Mono(r["ticket"].as_str().unwrap_or("").into()),
                        Cell::Pill(
                            score.to_string(),
                            if score >= 4 { Tone::Danger } else { Tone::Warn },
                        ),
                        Cell::Strong(r["subject"].as_str().unwrap_or("").into()),
                        Cell::Text(r["requester"].as_str().unwrap_or("").into()),
                        Cell::Pills(
                            r["hits"]
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(Value::as_str)
                                        .map(|h| (h.to_string(), Tone::Violet))
                                        .collect()
                                })
                                .unwrap_or_default(),
                        ),
                        Cell::Pill(
                            if sent {
                                f.t("gesendet", "sent").into()
                            } else {
                                f.t("offen", "not sent").into()
                            },
                            if sent { Tone::Ok } else { Tone::Neutral },
                        ),
                    ],
                )
            })
            .collect();
        let mut pick = None;
        widgets::card(ui, |ui| {
            for e in Table::new("desk_unhappy", &cols, &data)
                .filter_box(false)
                .show(ui)
            {
                let (TableEvent::Select(k) | TableEvent::Open(k)) = e;
                pick = Some(k);
            }
        });
        if let Some(k) = pick {
            self.open_ticket(&k);
            f.goto(V_UNSOLVED);
            self.last_view = V_UNSOLVED;
        }
    }

    fn view_macros(&mut self, ui: &mut Ui, f: &mut Frame) {
        widgets::page_header(
            ui,
            f.t("Makros", "Macros"),
            f.t(
                "Vorlagen für Antworten. Platzhalter: {{first_name}}, {{ticket}}, {{subject}}.",
                "Reply templates. Placeholders: {{first_name}}, {{ticket}}, {{subject}}.",
            ),
            |ui| {
                if widgets::primary(ui, f.t("Neues Makro", "New macro")).clicked() {
                    f.form("desk_macro_set", &[]);
                }
                if ui
                    .button(f.t("Als Liste", "As list"))
                    .on_hover_text("desk_macro_list")
                    .clicked()
                {
                    f.run_show("desk_macro_list", json!({}));
                }
            },
        );
        if self.macros.is_empty() {
            widgets::card(ui, |ui| {
                widgets::empty_state(
                    ui,
                    f.t("Keine Makros", "No macros"),
                    f.t("Lege ein Makro an.", "Create a macro."),
                )
            });
        }
        let de = f.lang_de;
        for m in &self.macros {
            widgets::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&m.name).family(semibold()).size(16.0));
                    if let Some(s) = m.set_status {
                        widgets::pill(ui, &format!("-> {}", status_label(s, de)), status_tone(s));
                    }
                    for t in &m.add_tags {
                        widgets::pill(ui, &format!("+{t}"), Tone::Neutral);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(f.t("Bearbeiten", "Edit")).clicked() {
                            let status = m.set_status.map(|s| s.as_str()).unwrap_or("");
                            f.form(
                                "desk_macro_set",
                                &[
                                    ("name", &m.name),
                                    ("body", &m.body),
                                    ("set_status", status),
                                    ("add_tags", &m.add_tags.join(",")),
                                ],
                            );
                        }
                    });
                });
                ui.label(RichText::new(&m.body).size(13.5));
            });
            ui.add_space(6.0);
        }
    }

    fn view_ingest(&mut self, ui: &mut Ui, f: &mut Frame) {
        widgets::page_header(ui, f.t("Mails einlesen", "Ingest mail"), f.t("Ein Ordner mit .eml-Dateien oder ein Maildir (cur/new). Doppelt eingelesene Mails werden erkannt und übersprungen.", "A folder of .eml files or a maildir (cur/new). Mails read twice are recognised and skipped."), |_| {});
        widgets::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.ingest_path)
                        .desired_width(460.0)
                        .hint_text("D:\\Mail\\Support"),
                );
                if ui
                    .button(f.t("Ordner wählen...", "Choose folder..."))
                    .clicked()
                {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.ingest_path = p.display().to_string();
                    }
                }
                if ui
                    .button(f.t("Datei wählen...", "Choose file..."))
                    .clicked()
                {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("E-Mail", &["eml"])
                        .pick_file()
                    {
                        self.ingest_path = p.display().to_string();
                    }
                }
            });
            ui.add_space(6.0);
            let ok = !self.ingest_path.trim().is_empty();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(ok, egui::Button::new(f.t("Probelauf", "Dry run")))
                    .clicked()
                {
                    f.run_show(
                        "desk_ingest",
                        json!({ "path": self.ingest_path.trim(), "dry_run": true }),
                    );
                }
                if ui
                    .add_enabled(
                        ok,
                        egui::Button::new(
                            RichText::new(f.t("Einlesen", "Ingest")).family(semibold()),
                        ),
                    )
                    .clicked()
                {
                    f.run_show("desk_ingest", json!({ "path": self.ingest_path.trim() }));
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new(f.t("Outlook: Mails in einen Ordner ziehen. Thunderbird: ImportExportTools NG, Export als .eml.", "Outlook: drag mails into a folder. Thunderbird: ImportExportTools NG, export as .eml.")).color(theme::current(ui.ctx()).text_weak));
        });
    }

    fn view_settings(&mut self, ui: &mut Ui, f: &mut Frame) {
        widgets::page_header(ui, f.t("Einstellungen", "Settings"), &self.db, |_| {});
        let c = &self.config;
        let get = |k: &str| c[k].as_str().unwrap_or("").to_string();
        widgets::card(ui, |ui| {
            ui.set_width(ui.available_width());
            widgets::section(ui, f.t("Support-Adresse", "Support address"), None);
            for (label, k) in [
                (f.t("Adresse", "Address"), "support_address"),
                (f.t("Absendername", "Sender name"), "support_name"),
                (f.t("Signatur", "Signature"), "signature"),
                (f.t("Postausgang", "Outbox"), "outbox"),
            ] {
                widgets::field(ui, label, |ui| {
                    ui.label(RichText::new(get(k)).family(egui::FontFamily::Monospace));
                });
            }
            if widgets::primary(ui, f.t("Ändern...", "Change...")).clicked() {
                f.form(
                    "desk_config",
                    &[
                        ("support_address", &get("support_address")),
                        ("support_name", &get("support_name")),
                        ("signature", &get("signature")),
                    ],
                );
            }
        });
        ui.add_space(10.0);
        widgets::card(ui, |ui| {
            ui.set_width(ui.available_width());
            widgets::section(ui, f.t("Daten", "Data"), None);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Status").clicked() {
                    f.run_show("desk_status", json!({}));
                }
                let empty = self.rows.is_empty() && self.solved.is_empty();
                if ui
                    .add_enabled(
                        empty,
                        egui::Button::new(f.t("Demo-Daten laden", "Load demo data")),
                    )
                    .on_disabled_hover_text("only on an empty desk")
                    .clicked()
                {
                    f.run(f.t("Demo-Daten", "Demo data"), "desk_demo_seed", json!({}));
                }
            });
        });
    }
}

fn message_bubble(ui: &mut Ui, m: &Message, de: bool, f: &mut Frame) {
    let t = theme::current(ui.ctx());
    let (tone, label) = match (m.direction.as_str(), m.draft, m.internal) {
        (_, _, true) | ("note", _, _) => (
            Tone::Warn,
            if de { "Interne Notiz" } else { "Internal note" },
        ),
        ("out", true, _) => (Tone::Violet, if de { "Entwurf" } else { "Draft" }),
        ("out", false, _) => (Tone::Accent, if de { "Antwort" } else { "Reply" }),
        _ => (Tone::Neutral, if de { "Kunde" } else { "Customer" }),
    };
    let (fg, bg) = theme::tone_colors(tone, t.dark);
    let fill = if tone == Tone::Neutral { t.surface } else { bg };
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(10)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let who = m.from_name.clone().unwrap_or_else(|| m.from_email.clone());
                widgets::avatar(ui, &who, 11.0);
                ui.label(RichText::new(&who).family(semibold()));
                widgets::pill(ui, label, tone);
                ui.label(RichText::new(dt(&m.date)).color(t.text_weak).size(12.5));
                if m.draft {
                    match m.sent_at {
                        Some(s) => {
                            widgets::pill(
                                ui,
                                &format!("{} {}", if de { "gesendet" } else { "sent" }, dt(&s)),
                                Tone::Ok,
                            );
                        }
                        None => {
                            if ui
                                .button(if de {
                                    "Als gesendet markieren"
                                } else {
                                    "Mark as sent"
                                })
                                .on_hover_text("after you sent it from your mail client")
                                .clicked()
                            {
                                f.run(
                                    if de {
                                        "Als gesendet markiert"
                                    } else {
                                        "Marked as sent"
                                    },
                                    "desk_reply_mark_sent",
                                    json!({ "message": m.id }),
                                );
                            }
                            if let Some(file) = &m.file {
                                if ui
                                    .button(if de {
                                        "Im Ordner zeigen"
                                    } else {
                                        "Show in folder"
                                    })
                                    .on_hover_text(file)
                                    .clicked()
                                {
                                    show_in_folder(file);
                                }
                            }
                        }
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(m.body.trim())
                    .color(if tone == Tone::Neutral { t.text } else { fg })
                    .size(14.5),
            );
        });
}

fn show_in_folder(file: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{file}"))
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = file;
    }
}

impl SuiteApp for DeskApp {
    fn name(&self) -> &'static str {
        "DeskCraft"
    }
    fn id(&self) -> &'static str {
        deskcraft_core::APP
    }
    fn tone(&self) -> Tone {
        Tone::Violet
    }
    fn version(&self) -> &'static str {
        deskcraft_core::VERSION
    }
    fn tools(&self) -> Vec<Value> {
        tools()
    }
    fn call(&mut self, tool: &str, args: &Value) -> Result<Value, String> {
        let cmd = Command::from_tool(tool, args)?;
        dispatch(&mut self.ctx, cmd)
    }
    fn reload(&mut self) -> Result<(), String> {
        self.now = now();
        let d = &self.ctx.desk;
        let open = d
            .tickets(
                &[Status::New, Status::Open, Status::Pending],
                None,
                None,
                None,
            )
            .map_err(|e| e.to_string())?;
        self.rows = open
            .into_iter()
            .map(|t| {
                let s = sla_json(&t, self.now);
                let g = |p: &str, k: &str| s[p][k].as_str().unwrap_or("").to_string();
                Row {
                    sla: Sla {
                        fr: g("first_response", "state"),
                        fr_due: g("first_response", "due"),
                        res: g("resolution", "state"),
                        res_due: g("resolution", "due"),
                    },
                    t,
                }
            })
            .collect();
        let mut solved = d
            .tickets(&[Status::Solved, Status::Closed], None, None, None)
            .map_err(|e| e.to_string())?;
        solved.sort_by_key(|t| std::cmp::Reverse(t.solved_at));
        solved.truncate(200);
        self.solved = solved;
        self.macros = d.macros().map_err(|e| e.to_string())?;
        self.config = dispatch(
            &mut self.ctx,
            Command::from_tool("desk_config", &json!({}))?,
        )?;
        let sig = dispatch(
            &mut self.ctx,
            Command::from_tool("desk_signals", &json!({ "emit": false }))?,
        )?;
        self.unhappy = sig["rows"].as_array().cloned().unwrap_or_default();
        if let Some(k) = self.open.as_ref().map(|o| o.key.clone()) {
            self.open_ticket(&k);
        }
        Ok(())
    }
    fn nav(&self, f: &Frame) -> Vec<NavEntry> {
        let n = |v: usize| self.list_for(v).len();
        let unsent = self
            .unhappy
            .iter()
            .filter(|r| r["sent_to_gaincraft"].is_null())
            .count();
        vec![
            NavEntry::new("\u{2709}", f.t("Alle offenen", "All unsolved"))
                .group(f.t("Ansichten", "Views"))
                .badge(n(V_UNSOLVED), Tone::Neutral),
            NavEntry::new("\u{2690}", f.t("Nicht zugewiesen", "Unassigned"))
                .badge(n(V_UNASSIGNED), Tone::Warn),
            NavEntry::new("\u{26A0}", f.t("Dringend und hoch", "Urgent and high"))
                .badge(n(V_URGENT), Tone::Danger),
            NavEntry::new("\u{23F1}", f.t("SLA in Gefahr", "SLA at risk"))
                .badge(n(V_SLA), Tone::Danger),
            NavEntry::new("\u{231B}", f.t("Wartend", "Pending")).badge(n(V_PENDING), Tone::Info),
            NavEntry::new("\u{2713}", f.t("Gelöst", "Recently solved")),
            NavEntry::new("\u{25A6}", f.t("Board", "Board")).group(f.t("Arbeit", "Work")),
            NavEntry::new("\u{2639}", f.t("Unzufrieden", "Unhappy customers"))
                .badge(unsent, Tone::Violet),
            NavEntry::new("\u{2630}", f.t("Makros", "Macros")),
            NavEntry::new("\u{21E9}", f.t("Mails einlesen", "Ingest mail"))
                .group(f.t("Daten", "Data")),
            NavEntry::new("\u{2699}", f.t("Einstellungen", "Settings")),
        ]
    }
    fn view(&mut self, ui: &mut Ui, view: usize, f: &mut Frame) {
        if view != self.last_view {
            self.open = None;
            self.last_view = view;
        }
        if self.open.is_some() && view <= V_BOARD {
            self.view_ticket(ui, f);
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt(("desk_view", view))
            .auto_shrink([false, false])
            .show(ui, |ui| match view {
                V_BOARD => self.view_board(ui, f),
                V_UNHAPPY => self.view_unhappy(ui, f),
                V_MACROS => self.view_macros(ui, f),
                V_INGEST => self.view_ingest(ui, f),
                V_SETTINGS => self.view_settings(ui, f),
                v => self.view_list(ui, v, f),
            });
    }
    fn top_actions(&mut self, ui: &mut Ui, f: &mut Frame) {
        if widgets::primary(ui, f.t("Mails einlesen", "Ingest mail")).clicked() {
            f.goto(V_INGEST);
        }
    }
    fn bus(&self) -> &Bus {
        &self.ctx.bus
    }
    fn wiring(&self) -> Wiring {
        (
            vec![],
            vec![
                (
                    "ticket.escalated",
                    "to taskcraft: the ticket becomes an issue",
                ),
                (
                    "signal.negative",
                    "to gaincraft: unhappy customer lowers the health score",
                ),
            ],
        )
    }
    fn prefill(&self) -> Vec<(String, String)> {
        self.open
            .iter()
            .map(|o| ("ticket".to_string(), o.key.clone()))
            .collect()
    }
    fn status_info(&self) -> String {
        format!("{}  |  {} open", self.db, self.rows.len())
    }
    fn select(&mut self, key: &str) {
        self.open_ticket(key);
    }
    fn smoke_tool(&self) -> &'static str {
        "desk_status"
    }
}

fn default_db() -> PathBuf {
    if let Ok(p) = std::env::var("DESKCRAFT_DB") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("deskcraft")
        .join("deskcraft.db")
}

fn open(db: &Path) -> Result<DeskApp, String> {
    let desk = Desk::open(db).map_err(|e| e.to_string())?;
    let actor = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "desktop".into());
    let outbox = db
        .parent()
        .map(|p| p.join("outbox"))
        .unwrap_or_else(|| PathBuf::from("outbox"));
    let ctx = Ctx {
        desk,
        confinement: Confinement(None),
        bus: Bus::new(Bus::default_root()),
        outbox,
        actor,
    };
    Ok(DeskApp::new(ctx, db.display().to_string()))
}

fn main() -> std::process::ExitCode {
    app::main_with(default_db(), open, "desk_demo_seed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_biz_ui::app::Shell;
    use egui_kittest::Harness;

    fn shell(tag: &str) -> (Shell<DeskApp>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("deskcraft-app-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ctx = Ctx {
            desk: Desk::open_in_memory().unwrap(),
            confinement: Confinement(None),
            bus: Bus::new(dir.join("bus")),
            outbox: dir.join("outbox"),
            actor: "Anna Mockova".into(),
        };
        let mut app = DeskApp::new(ctx, ":memory:".into());
        app.call("desk_demo_seed", &json!({})).unwrap();
        (Shell::new(app, None), dir)
    }

    #[test]
    fn every_tool_has_a_place_in_the_window() {
        craft_biz_ui::coverage::check(&tools(), COVERAGE).unwrap();
    }

    #[test]
    fn every_view_and_the_ticket_page_render_without_panic() {
        let (sh, dir) = shell("render");
        let key = sh.app.rows.first().map(|r| r.t.key.clone()).unwrap();
        let mut h = Harness::builder()
            .with_size([1440.0, 900.0])
            .build_ui_state(|ui, sh: &mut Shell<DeskApp>| sh.show(ui), sh);
        for v in 0..11 {
            h.state_mut().view = v;
            h.run_steps(3);
        }
        h.state_mut().view = V_UNSOLVED;
        h.run_steps(2);
        h.state_mut().app.open_ticket(&key);
        h.run_steps(3);
        craft_biz_ui::theme::apply(&h.ctx, true);
        h.run_steps(2);
        assert!(h.state().last_error.is_none(), "{:?}", h.state().last_error);
        assert!(h.state().app.open.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reply_draft_note_status_and_mark_sent_go_through_dispatch() {
        let (mut sh, dir) = shell("reply");
        let key = sh.app.rows.first().map(|r| r.t.key.clone()).unwrap();
        sh.app.open_ticket(&key);
        sh.run_tool(
            "desk_reply_draft",
            json!({ "ticket": key, "body": "Danke, wir kümmern uns (Test)." }),
        );
        assert!(sh.last_error.is_none(), "{:?}", sh.last_error);
        let draft = sh
            .app
            .open
            .as_ref()
            .and_then(|o| {
                o.messages
                    .iter()
                    .find(|m| m.draft && m.sent_at.is_none())
                    .map(|m| m.id)
            })
            .unwrap();
        assert!(
            sh.app
                .open
                .as_ref()
                .and_then(|o| o.messages.iter().find(|m| m.id == draft))
                .and_then(|m| m.file.clone())
                .is_some_and(|f| Path::new(&f).exists()),
            ".eml written to the outbox"
        );
        sh.run_tool("desk_reply_mark_sent", json!({ "message": draft }));
        assert!(sh
            .app
            .open
            .as_ref()
            .unwrap()
            .messages
            .iter()
            .any(|m| m.id == draft && m.sent_at.is_some()));
        sh.run_tool(
            "desk_ticket_update",
            json!({ "ticket": key, "status": "pending", "note": "Kunde ruft zurück (Test)" }),
        );
        assert_eq!(sh.app.ticket(&key).map(|t| t.status), Some(Status::Pending));
        assert!(sh
            .app
            .open
            .as_ref()
            .unwrap()
            .history
            .iter()
            .any(|h| h.actor == "Anna Mockova" && h.field == "status"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn board_drag_changes_status_and_refuses_new() {
        let (mut sh, dir) = shell("board");
        sh.view = V_BOARD;
        let key = sh
            .app
            .rows
            .iter()
            .find(|r| r.t.status == Status::Open || r.t.status == Status::New)
            .map(|r| r.t.key.clone())
            .unwrap();
        let mut h = Harness::builder()
            .with_size([1600.0, 900.0])
            .build_ui_state(|ui, sh: &mut Shell<DeskApp>| sh.show(ui), sh);
        h.run_steps(3);
        let drag = |h: &mut Harness<'_, Shell<DeskApp>>, col: &str| {
            let card = kanban::card_rect(&h.ctx, "desk_board", &key).unwrap();
            let target = kanban::cell_rect(&h.ctx, "desk_board", col, "").unwrap();
            h.hover_at(card.center());
            h.run_steps(1);
            h.drag_at(card.center());
            h.run_steps(1);
            for i in 1..=8 {
                h.hover_at(card.center() + (target.center() - card.center()) * (i as f32 / 8.0));
                h.run_steps(1);
            }
            h.drop_at(target.center());
            h.run_steps(3);
        };
        drag(&mut h, "pending");
        assert_eq!(
            h.state().app.ticket(&key).map(|t| t.status),
            Some(Status::Pending),
            "{:?}",
            h.state().last_error
        );
        drag(&mut h, "new");
        assert_eq!(
            h.state().app.ticket(&key).map(|t| t.status),
            Some(Status::Pending),
            "drop on New refused"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
