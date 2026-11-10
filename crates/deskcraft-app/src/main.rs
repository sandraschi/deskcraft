//! `deskcraft-app`: Native eframe/egui desktop workstation for DeskCraft.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::Utc;
use craft_biz::bus::Bus;
use craft_biz::confine::Confinement;
use deskcraft_core::{dispatch, Command, Ctx};
use deskcraft_store::{Desk, Message, Priority, Status, Ticket};
use eframe::egui::{self, Color32, RichText};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewFilter {
    Open,
    Pending,
    SlaBreach,
    Solved,
    All,
}

struct App {
    ctx: Ctx,
    view: ViewFilter,
    tickets: Vec<Ticket>,
    selected_ticket: Option<Ticket>,
    selected_messages: Vec<Message>,
    reply_draft_body: String,
    breached_keys: Vec<String>,
    status_msg: String,
}

impl App {
    fn new(ctx: Ctx) -> Self {
        let mut app = Self {
            ctx,
            view: ViewFilter::Open,
            tickets: Vec::new(),
            selected_ticket: None,
            selected_messages: Vec::new(),
            reply_draft_body: String::new(),
            breached_keys: Vec::new(),
            status_msg: "Ready".to_string(),
        };
        app.reload_tickets();
        app
    }

    fn reload_tickets(&mut self) {
        let now_dt = Utc::now().naive_utc();
        let mut breaches = Vec::new();
        if let Ok(sla_res) = deskcraft_core::sla_status(&self.ctx.desk, now_dt) {
            if let Some(rows) = sla_res.get("rows").and_then(|v| v.as_array()) {
                for r in rows {
                    let first = r.get("first_response").and_then(|v| v.as_str()).unwrap_or("");
                    let resol = r.get("resolution").and_then(|v| v.as_str()).unwrap_or("");
                    if first == "breached" || resol == "breached" {
                        if let Some(k) = r.get("ticket").and_then(|v| v.as_str()) {
                            breaches.push(k.to_string());
                        }
                    }
                }
            }
        }
        self.breached_keys = breaches;

        let filter = match self.view {
            ViewFilter::Open => vec![Status::New, Status::Open],
            ViewFilter::Pending => vec![Status::Pending],
            ViewFilter::SlaBreach => vec![Status::New, Status::Open, Status::Pending],
            ViewFilter::Solved => vec![Status::Solved, Status::Closed],
            ViewFilter::All => Vec::new(),
        };

        if let Ok(ts) = self.ctx.desk.tickets(&filter, None, None, None) {
            if self.view == ViewFilter::SlaBreach {
                self.tickets = ts
                    .into_iter()
                    .filter(|t| self.breached_keys.contains(&t.key))
                    .collect();
            } else {
                self.tickets = ts;
            }
            if self.selected_ticket.is_none() && !self.tickets.is_empty() {
                let first = self.tickets[0].clone();
                self.select_ticket(first);
            }
        }
    }

    fn select_ticket(&mut self, ticket: Ticket) {
        if let Ok(msgs) = self.ctx.desk.messages(ticket.id) {
            self.selected_messages = msgs;
        }
        self.selected_ticket = Some(ticket);
    }

    fn send_reply(&mut self) {
        let Some(ref t) = self.selected_ticket else {
            return;
        };
        let body = self.reply_draft_body.trim();
        if body.is_empty() {
            return;
        }

        match dispatch(
            &mut self.ctx,
            Command::ReplyDraft {
                ticket: t.key.clone(),
                body: Some(body.to_string()),
                macro_name: None,
            },
        ) {
            Ok(_) => {
                self.status_msg = format!("Reply drafted for {}", t.key);
                self.reply_draft_body.clear();
                if let Ok(msgs) = self.ctx.desk.messages(t.id) {
                    self.selected_messages = msgs;
                }
            }
            Err(e) => {
                self.status_msg = format!("Reply error: {e}");
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.horizontal(|ui| {
            ui.heading(
                RichText::new("🎧 DeskCraft Support Studio")
                    .color(Color32::from_rgb(56, 189, 248))
                    .strong(),
            );
            ui.separator();

            if ui
                .selectable_label(self.view == ViewFilter::Open, "📥 Open")
                .clicked()
            {
                self.view = ViewFilter::Open;
                self.reload_tickets();
            }
            if ui
                .selectable_label(self.view == ViewFilter::Pending, "⏳ Pending")
                .clicked()
            {
                self.view = ViewFilter::Pending;
                self.reload_tickets();
            }
            if ui
                .selectable_label(
                    self.view == ViewFilter::SlaBreach,
                    format!("🚨 SLA Breaches ({})", self.breached_keys.len()),
                )
                .clicked()
            {
                self.view = ViewFilter::SlaBreach;
                self.reload_tickets();
            }
            if ui
                .selectable_label(self.view == ViewFilter::Solved, "✅ Solved")
                .clicked()
            {
                self.view = ViewFilter::Solved;
                self.reload_tickets();
            }
            if ui
                .selectable_label(self.view == ViewFilter::All, "📋 All")
                .clicked()
            {
                self.view = ViewFilter::All;
                self.reload_tickets();
            }

            ui.separator();
            if ui.button("🔄 Refresh").clicked() {
                self.reload_tickets();
                self.status_msg = "Tickets reloaded".to_string();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("Agent: {}", self.ctx.actor));
            });
        });

        ui.separator();

        ui.horizontal(|ui| {
            // Left column: Tickets List
            ui.vertical(|ui| {
                ui.set_width(320.0);
                ui.heading(RichText::new("Ticket Inbox").size(15.0));
                ui.separator();

                let mut to_select = None;
                egui::ScrollArea::vertical()
                    .id_salt("ticket_list_scroll")
                    .show(ui, |ui| {
                        for t in &self.tickets {
                            let is_sel = self
                                .selected_ticket
                                .as_ref()
                                .map(|st| st.id == t.id)
                                .unwrap_or(false);

                            let prio_color = match t.priority {
                                Priority::Urgent => Color32::from_rgb(231, 76, 60),
                                Priority::High => Color32::from_rgb(230, 126, 34),
                                Priority::Normal => Color32::from_rgb(52, 152, 219),
                                Priority::Low => Color32::from_rgb(149, 165, 166),
                            };

                            let is_breached = self.breached_keys.contains(&t.key);
                            let req = t
                                .requester_name
                                .as_deref()
                                .unwrap_or(&t.requester_email);

                            ui.group(|ui| {
                                if ui
                                    .selectable_label(
                                        is_sel,
                                        format!("{}: {}", t.key, t.subject),
                                    )
                                    .clicked()
                                {
                                    to_select = Some(t.clone());
                                }
                                ui.horizontal(|ui| {
                                    ui.colored_label(
                                        prio_color,
                                        format!("[{}]", t.priority.as_str()),
                                    );
                                    ui.label(req);
                                    if is_breached {
                                        ui.colored_label(
                                            Color32::from_rgb(231, 76, 60),
                                            "BREACH",
                                        );
                                    }
                                });
                            });
                            ui.add_space(2.0);
                        }
                    });

                if let Some(t) = to_select {
                    self.select_ticket(t);
                }
            });

            ui.separator();

            // Right column: Selected Ticket Conversation Thread & Composer
            ui.vertical(|ui| {
                if let Some(ref t) = self.selected_ticket {
                    ui.horizontal(|ui| {
                        ui.heading(RichText::new(format!("{}: {}", t.key, t.subject)).size(18.0));
                        ui.separator();
                        ui.label(format!("Status: {}", t.status.as_str()));
                        ui.label(format!("Priority: {}", t.priority.as_str()));
                    });

                    ui.separator();

                    ui.label(RichText::new("Conversation Thread:").strong());
                    egui::ScrollArea::vertical()
                        .id_salt("msg_thread_scroll")
                        .max_height(340.0)
                        .show(ui, |ui| {
                            for m in &self.selected_messages {
                                let is_out = m.direction == "outbound";
                                let (label_color, label) = if is_out {
                                    (Color32::from_rgb(46, 204, 113), "Agent Reply")
                                } else {
                                    (Color32::from_rgb(52, 152, 219), "Customer Message")
                                };

                                let sender = m
                                    .from_name
                                    .as_deref()
                                    .unwrap_or(&m.from_email);

                                ui.group(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.colored_label(label_color, label);
                                        ui.label(format!("From: {sender}"));
                                        ui.label(format!("At: {}", m.date));
                                    });
                                    ui.add_space(4.0);
                                    ui.label(&m.body);
                                });
                                ui.add_space(6.0);
                            }
                        });

                    ui.separator();

                    // Reply Composer
                    ui.label(RichText::new("Draft Reply:").strong());
                    ui.add(
                        egui::TextEdit::multiline(&mut self.reply_draft_body)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    );

                    ui.horizontal(|ui| {
                        if ui.button("✉️ Draft & Queue Reply").clicked() {
                            self.send_reply();
                        }
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label("Select a ticket from the inbox to view thread.");
                    });
                }
            });
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(format!(
                "Tickets in View: {} | Selected: {}",
                self.tickets.len(),
                self.selected_ticket.as_ref().map(|t| t.key.as_str()).unwrap_or("-")
            ));
            ui.separator();
            ui.label(&self.status_msg);
        });
    }
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

fn default_outbox() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("deskcraft").join("outbox")
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--smoke") {
        println!("smoke: ok (deskcraft-app)");
        return Ok(());
    }

    let db_path = default_db();
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let outbox = default_outbox();
    let _ = std::fs::create_dir_all(&outbox);

    let desk = match Desk::open(&db_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to open desk database at {}: {e}", db_path.display());
            std::process::exit(1);
        }
    };

    let ctx = Ctx {
        desk,
        confinement: Confinement(None),
        bus: Bus::new(Bus::default_root()),
        outbox,
        actor: std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "desktop".to_string()),
    };

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 760.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title("DeskCraft Support Studio"),
        ..Default::default()
    };

    eframe::run_native(
        "DeskCraft",
        native_options,
        Box::new(|_cc| Ok(Box::new(App::new(ctx)))),
    )
}
