use super::*;
use chrono::NaiveDate;

struct Env {
    ctx: Ctx,
    dir: PathBuf,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn env(tag: &str) -> Env {
    let dir = std::env::temp_dir().join(format!("deskcraft-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ctx = Ctx {
        desk: Desk::open_in_memory().unwrap(),
        confinement: Confinement::roots(std::slice::from_ref(&dir)).unwrap(),
        bus: Bus::new(dir.join("bus")),
        outbox: dir.join("outbox"),
        actor: "test".into(),
    };
    Env { ctx, dir }
}

fn call(c: &mut Ctx, name: &str, a: Value) -> Result<Value, String> {
    let cmd = Command::from_tool(name, &a)?;
    dispatch(c, cmd)
}

fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 11, d)
        .unwrap()
        .and_hms_opt(h, m, 0)
        .unwrap()
}

#[test]
fn catalog_tools_are_routable() {
    assert_eq!(tools().len(), 14);
    for t in tools() {
        let name = t["name"].as_str().unwrap();
        if let Err(err) = Command::from_tool(name, &json!({})) {
            assert!(!err.starts_with("unknown tool"), "{name}");
        }
    }
}

#[test]
fn demo_threads_all_three_ways() {
    let mut env = env("demo");
    let c = &mut env.ctx;
    let seeded = call(c, "desk_demo_seed", json!({})).unwrap();
    let outcomes: Vec<&str> = seeded["ingest"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        outcomes,
        vec![
            "NewTicket(\"D-1\")",
            "Outgoing(\"D-1\")",
            "Appended(\"D-1\")",
            "Appended(\"D-1\")",
            "NewTicket(\"D-2\")",
            "NewTicket(\"D-3\")"
        ]
    );
    let t1 = c.desk.ticket("D-1").unwrap();
    assert_eq!(t1.subject, "Drucker im Büro druckt nicht");
    assert_eq!(t1.message_count, 4);
    assert_eq!(
        t1.first_response_at,
        Some(at(2, 10, 5)),
        "reply found in ingest stops the clock"
    );
    assert_eq!(
        t1.status,
        Status::Open,
        "a sent reply moves new to open (regression)"
    );
    assert_eq!(
        c.desk.ticket("D-2").unwrap().status,
        Status::New,
        "unanswered stays new"
    );
    assert_eq!(
        c.desk.ticket("D-3").unwrap().priority,
        Priority::High,
        "DRINGEND in subject"
    );
    assert!(call(c, "desk_demo_seed", json!({})).is_err());
}

#[test]
fn ingest_folder_is_idempotent_and_confined() {
    let mut env = env("ingest");
    let c = &mut env.ctx;
    let maildir = env.dir.join("Inbox").join("cur");
    std::fs::create_dir_all(&maildir).unwrap();
    std::fs::write(
        maildir.join("1700000000.M1P1.goliath!2,S"),
        demo_mail(
            "A Mock <a@mock.example>",
            "Frage",
            "Mon, 9 Nov 2026 09:00:00 +0100",
            "q1@mock.example",
            "",
            "Hallo",
        ),
    )
    .unwrap();
    std::fs::write(
        env.dir.join("loose.eml"),
        demo_mail(
            "B Mock <b@mock.example>",
            "Andere Frage",
            "Mon, 9 Nov 2026 09:30:00 +0100",
            "q2@mock.example",
            "",
            "Hi",
        ),
    )
    .unwrap();
    std::fs::write(env.dir.join("notes.txt"), "not mail").unwrap();
    std::fs::write(env.dir.join("broken.eml"), "garbage without headers").unwrap();
    let dry = call(
        c,
        "desk_ingest",
        json!({ "path": env.dir.display().to_string(), "dry_run": true }),
    )
    .unwrap();
    assert_eq!(dry["new_tickets"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        c.desk.stats().unwrap().messages,
        0,
        "dry run stores nothing"
    );
    let r = call(
        c,
        "desk_ingest",
        json!({ "path": env.dir.display().to_string() }),
    )
    .unwrap();
    assert_eq!(
        (
            r["files"].as_u64(),
            r["new_tickets"].as_array().map(Vec::len),
            r["errors"].as_array().map(Vec::len)
        ),
        (Some(3), Some(2), Some(1))
    );
    let again = call(
        c,
        "desk_ingest",
        json!({ "path": env.dir.display().to_string() }),
    )
    .unwrap();
    assert_eq!(
        (
            again["duplicates"].as_u64(),
            again["new_tickets"].as_array().map(Vec::len)
        ),
        (Some(2), Some(0))
    );
    assert!(call(c, "desk_ingest", json!({ "path": "C:/Windows" }))
        .unwrap_err()
        .contains("outside"));
}

#[test]
fn reply_draft_is_a_threaded_eml_and_reply_reopens() {
    let mut env = env("reply");
    let c = &mut env.ctx;
    call(c, "desk_demo_seed", json!({})).unwrap();
    let r = call(
        c,
        "desk_reply_draft",
        json!({ "ticket": "D-2", "macro": "geloest" }),
    )
    .unwrap();
    assert_eq!(r["status"], "solved", "macro sets status");
    let file = PathBuf::from(r["file"].as_str().unwrap());
    let raw = std::fs::read(&file).unwrap();
    let back = mail::parse(&raw, None, at(9, 12, 0)).unwrap();
    assert_eq!(back.subject, "Re: Invoice question [D-2]");
    assert_eq!(back.in_reply_to.as_deref(), Some("i1@mockingbird.example"));
    assert!(back.body.starts_with("Hallo Mia,"));
    assert!(back.body.contains("Mockdesk Support (DEMO)"), "signature");
    assert!(
        c.desk.ticket("D-2").unwrap().first_response_at.is_none(),
        "draft is not a response"
    );
    call(
        c,
        "desk_reply_mark_sent",
        json!({ "message": r["message"], "at": "2026-11-09 12:00" }),
    )
    .unwrap();
    assert_eq!(
        c.desk.ticket("D-2").unwrap().first_response_at,
        Some(at(9, 12, 0))
    );
    // Customer answers our reply: thread via In-Reply-To, ticket reopens.
    let answer = demo_mail(
        "Mia Mockerl <mia@mockingbird.example>",
        "Re: Invoice question [D-2]",
        "Mon, 9 Nov 2026 13:00:00 +0100",
        "i2@mockingbird.example",
        &format!("In-Reply-To: <{}>\r\n", back.message_id.unwrap()),
        "Still wrong.",
    );
    assert_eq!(
        ingest_bytes(&c.desk, answer.as_bytes(), None).unwrap(),
        Outcome::Appended("D-2".into())
    );
    assert_eq!(c.desk.ticket("D-2").unwrap().status, Status::Open);
    // Closed tickets do not reopen: a new ticket with a note.
    call(
        c,
        "desk_ticket_update",
        json!({ "ticket": "D-2", "status": "closed" }),
    )
    .unwrap();
    let late = demo_mail(
        "Mia Mockerl <mia@mockingbird.example>",
        "Re: Invoice question [D-2]",
        "Mon, 9 Nov 2026 15:00:00 +0100",
        "i3@mockingbird.example",
        "In-Reply-To: <i2@mockingbird.example>\r\n",
        "One more thing.",
    );
    assert_eq!(
        ingest_bytes(&c.desk, late.as_bytes(), None).unwrap(),
        Outcome::NewTicket("D-4".into())
    );
    assert!(call(
        c,
        "desk_reply_draft",
        json!({ "ticket": "D-2", "body": "x" })
    )
    .is_err());
}

#[test]
fn sla_escalation_and_signals() {
    let mut env = env("sla");
    let c = &mut env.ctx;
    call(c, "desk_demo_seed", json!({})).unwrap();
    // As of Monday 9.11. 12:00: D-2 (normal, opened Thu 16:40) used 20+540+240 = 800 > 480 -> breached.
    let s = sla_status(&c.desk, at(9, 12, 0)).unwrap();
    let d2 = s["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["ticket"] == "D-2")
        .unwrap();
    assert_eq!(d2["first_response"], "breached");
    let d1 = s["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["ticket"] == "D-1")
        .unwrap();
    assert_eq!(d1["first_response"], "met");
    let e1 = call(
        c,
        "desk_escalate",
        json!({ "ticket": "D-3", "details": "Shop down since 07:50", "project": "OPS" }),
    )
    .unwrap();
    assert!(e1["event"].is_string());
    assert!(call(c, "desk_escalate", json!({ "ticket": "D-3" }))
        .unwrap_err()
        .contains("already"));
    let ev = c.bus.pending("taskcraft").unwrap();
    assert_eq!(
        (
            ev[0].kind.as_str(),
            ev[0].payload["priority"].as_str(),
            ev[0].payload["project"].as_str()
        ),
        ("ticket.escalated", Some("high"), Some("OPS"))
    );
    assert!(ev[0].payload["details"]
        .as_str()
        .unwrap()
        .contains("Frechheit"));
    assert!(c
        .desk
        .ticket("D-3")
        .unwrap()
        .tags
        .contains(&"escalated".to_string()));
    let sig = call(c, "desk_signals", json!({ "emit": true })).unwrap();
    assert_eq!(sig["rows"].as_array().map(Vec::len), Some(1));
    assert_eq!(sig["rows"][0]["ticket"], "D-3");
    assert_eq!(
        call(c, "desk_signals", json!({ "emit": true })).unwrap()["emitted"],
        0,
        "once per ticket"
    );
    let g = c.bus.pending("gaincraft").unwrap();
    assert_eq!(
        (g.len(), g[0].payload["account"].as_str()),
        (1, Some("mockinger.example"))
    );
    assert_eq!(negativity("Alles super, danke!").0, 0);
}
