use super::*;
use chrono::NaiveDate;

fn at(d: u32, h: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 11, d)
        .unwrap()
        .and_hms_opt(h, 0, 0)
        .unwrap()
}

fn msg(id: &str, hash: &str) -> NewMessage {
    NewMessage {
        message_id: Some(id.into()),
        from_email: "Kunde@Mock.at".into(),
        subject: "Drucker".into(),
        date: at(9, 9),
        body: "geht nicht".into(),
        sha256: hash.into(),
        ..Default::default()
    }
}

#[test]
fn keys_parse() {
    assert_eq!(parse_key("D-12").unwrap(), 12);
    assert_eq!(parse_key("#12").unwrap(), 12);
    assert_eq!(parse_key("d12").unwrap(), 12);
    assert!(parse_key("X-1").is_err());
    assert!(parse_key("D-0").is_err());
}

#[test]
fn messages_are_stored_once() {
    let d = Desk::open_in_memory().unwrap();
    let t = d
        .create_ticket(
            "Drucker",
            "Kunde@Mock.at",
            Some("Joe Mocky"),
            at(9, 9),
            Priority::Normal,
            "test",
        )
        .unwrap();
    assert_eq!(
        (t.key.as_str(), t.requester_email.as_str(), t.status),
        ("D-1", "kunde@mock.at", Status::New)
    );
    assert!(d
        .add_message(t.id, "in", &msg("<a@x>", "h1"), false, None)
        .unwrap()
        .is_some());
    assert!(
        d.add_message(t.id, "in", &msg("<a@x>", "h2"), false, None)
            .unwrap()
            .is_none(),
        "same Message-ID"
    );
    let no_id = NewMessage {
        message_id: None,
        ..msg("", "h3")
    };
    assert!(d
        .add_message(t.id, "in", &no_id, false, None)
        .unwrap()
        .is_some());
    assert!(
        d.add_message(t.id, "in", &no_id, false, None)
            .unwrap()
            .is_none(),
        "same bytes"
    );
    assert_eq!(d.ticket("D-1").unwrap().message_count, 2);
    assert_eq!(
        d.ticket_by_message_ids(&["<zz>".into(), "<a@x>".into()])
            .unwrap()
            .map(|t| t.id),
        Some(1)
    );
    assert!(d
        .ticket_by_subject("KUNDE@mock.at", "drucker", at(1, 0))
        .unwrap()
        .is_some());
    assert!(d
        .ticket_by_subject("other@mock.at", "Drucker", at(1, 0))
        .unwrap()
        .is_none());
}

#[test]
fn status_history_and_closed_is_final() {
    let d = Desk::open_in_memory().unwrap();
    let t = d
        .create_ticket("x", "a@b.at", None, at(9, 9), Priority::Low, "test")
        .unwrap();
    let t = d
        .set_status(&t, Status::Solved, "agent", at(9, 12))
        .unwrap();
    assert_eq!(t.solved_at, Some(at(9, 12)));
    let t = d.set_status(&t, Status::Open, "agent", at(9, 13)).unwrap();
    assert!(t.solved_at.is_none(), "reopened");
    let t = d.set_priority(&t, Priority::Urgent, "agent").unwrap();
    let t = d
        .set_tags(
            &t,
            &["Drucker".into(), "drucker".into(), " vip ".into()],
            "agent",
        )
        .unwrap();
    assert_eq!(t.tags, vec!["drucker", "vip"]);
    assert!(d.set_tags(&t, &["no spaces".into()], "agent").is_err());
    let t = d
        .set_status(&t, Status::Closed, "agent", at(9, 14))
        .unwrap();
    assert!(matches!(
        d.set_status(&t, Status::Open, "agent", at(9, 15)),
        Err(DeskError::State(_))
    ));
    let fields: Vec<String> = d
        .history(t.id)
        .unwrap()
        .into_iter()
        .map(|h| h.field)
        .collect();
    assert_eq!(
        fields,
        vec!["created", "status", "status", "priority", "tags", "status"]
    );
}

#[test]
fn drafts_and_first_response() {
    let d = Desk::open_in_memory().unwrap();
    let t = d
        .create_ticket("x", "a@b.at", None, at(9, 9), Priority::Normal, "test")
        .unwrap();
    let out = NewMessage {
        message_id: Some("<r1@desk>".into()),
        from_email: "support@mock.at".into(),
        subject: "Re: x [D-1]".into(),
        date: at(9, 10),
        body: "Hallo".into(),
        ..Default::default()
    };
    let m = d
        .add_message(t.id, "out", &out, true, Some("outbox/r1.eml"))
        .unwrap()
        .unwrap();
    assert!(m.draft);
    assert!(
        d.ticket_by_id(t.id).unwrap().first_response_at.is_none(),
        "a draft is not a response"
    );
    let sent = d.mark_sent(m.id, at(9, 11), "agent").unwrap();
    assert_eq!(sent.sent_at, Some(at(9, 11)));
    assert_eq!(
        d.ticket_by_id(t.id).unwrap().first_response_at,
        Some(at(9, 11))
    );
    assert!(d.mark_sent(m.id, at(9, 12), "agent").is_err(), "only once");
    assert_eq!(d.thread_ids(t.id).unwrap(), vec!["<r1@desk>"]);
}

#[test]
fn macros_roundtrip() {
    let d = Desk::open_in_memory().unwrap();
    let m = d
        .set_macro(
            "Danke-Geloest",
            "Danke {{name}}",
            Some(Status::Solved),
            &["solved-by-macro".into()],
        )
        .unwrap();
    assert_eq!(
        (m.name.as_str(), m.set_status),
        ("danke-geloest", Some(Status::Solved))
    );
    assert!(d.set_macro("x y", "b", None, &[]).is_err());
    assert_eq!(d.macros().unwrap().len(), 1);
}
