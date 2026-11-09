//! E-mail in and out: parse RFC 5322/MIME with `mail-parser`, normalize
//! subjects, find ticket tags, and write RFC 5322 reply drafts.
//!
//! Message-IDs are stored without angle brackets, as `mail-parser` returns them.

use chrono::{DateTime, Local, NaiveDateTime};
use deskcraft_store::NewMessage;
use mail_parser::{HeaderValue, MessageParser};
use sha2::{Digest, Sha256};

pub const MAX_BODY_CHARS: usize = 100_000;

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn ids(v: &HeaderValue) -> Vec<String> {
    match v {
        HeaderValue::Text(t) => vec![t.trim_matches(['<', '>', ' ']).to_string()],
        HeaderValue::TextList(l) => l
            .iter()
            .map(|t| t.trim_matches(['<', '>', ' ']).to_string())
            .collect(),
        _ => Vec::new(),
    }
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect()
}

/// Parse raw message bytes. `now` is used when the mail has no usable Date.
pub fn parse(raw: &[u8], source: Option<&str>, now: NaiveDateTime) -> Result<NewMessage, String> {
    let msg = MessageParser::default()
        .parse(raw)
        .ok_or("not an e-mail message")?;
    let from = msg
        .from()
        .and_then(|a| a.first())
        .ok_or("no From address")?;
    let from_email = from
        .address()
        .ok_or("From has no address")?
        .trim()
        .to_lowercase();
    if !from_email.contains('@') {
        return Err(format!(
            "From address {from_email:?} is not an e-mail address"
        ));
    }
    let to = msg
        .to()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.address())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let date = msg
        .date()
        .and_then(|d| DateTime::from_timestamp(d.to_timestamp(), 0))
        .map(|d| d.with_timezone(&Local).naive_local())
        .unwrap_or(now);
    let mut body: String = msg
        .body_text(0)
        .map(|b| b.replace("\r\n", "\n"))
        .unwrap_or_default();
    if body.chars().count() > MAX_BODY_CHARS {
        body = body.chars().take(MAX_BODY_CHARS).collect::<String>() + "\n[... truncated]";
    }
    let mut references = ids(msg.references());
    references.reverse(); // newest first for threading
    Ok(NewMessage {
        message_id: msg
            .message_id()
            .map(|m| m.trim_matches(['<', '>', ' ']).to_string())
            .filter(|m| !m.is_empty()),
        in_reply_to: ids(msg.in_reply_to()).into_iter().next(),
        references,
        from_email,
        from_name: from
            .name()
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty()),
        to,
        subject: msg.subject().unwrap_or("").trim().to_string(),
        date,
        body: body.trim().to_string(),
        sha256: sha256_hex(raw),
        source: source.map(str::to_string),
    })
}

/// Strip reply/forward prefixes (Re, AW, Fwd, WG, ...) and our `[D-n]` tags.
pub fn normalize_subject(s: &str) -> String {
    let mut t = strip_tags(s).trim().to_string();
    loop {
        let lower = t.to_lowercase();
        let hit = [
            "re:", "aw:", "antw:", "fwd:", "fw:", "wg:", "sv:", "vs:", "re :", "aw :",
        ]
        .iter()
        .find(|p| lower.starts_with(*p));
        match hit {
            Some(p) => t = t[p.len()..].trim().to_string(),
            None => break,
        }
    }
    if t.is_empty() {
        "(no subject)".into()
    } else {
        t
    }
}

fn strip_tags(s: &str) -> String {
    let mut out = s.to_string();
    while let Some(i) = out.find("[D-") {
        match out[i..].find(']') {
            Some(j) if out[i + 3..i + j].chars().all(|c| c.is_ascii_digit()) && j > 3 => {
                out.replace_range(i..=i + j, "")
            }
            _ => break,
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Ticket number from a `[D-12]` tag in a subject.
pub fn ticket_tag(s: &str) -> Option<i64> {
    let i = s.find("[D-")?;
    let j = s[i..].find(']')?;
    s[i + 3..i + j].parse().ok()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (k, shift) in [18u32, 12, 6, 0].iter().enumerate() {
            if k <= chunk.len() {
                out.push(char::from(B64[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// RFC 2047 encoded-word when the text is not plain ASCII.
pub fn header_text(s: &str) -> String {
    let clean: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if clean.is_ascii() {
        clean
    } else {
        format!("=?utf-8?B?{}?=", base64(clean.as_bytes()))
    }
}

fn mailbox(name: Option<&str>, addr: &str) -> String {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) if n.is_ascii() && !n.contains(['"', ',', '<', '>', '(', ')', ';', ':', '@']) => {
            format!("{n} <{addr}>")
        }
        Some(n) if n.is_ascii() => format!("\"{}\" <{addr}>", n.replace('"', "'")),
        Some(n) => format!("{} <{addr}>", header_text(n)),
        None => addr.to_string(),
    }
}

pub struct Draft<'a> {
    pub message_id: &'a str,
    pub from_name: Option<&'a str>,
    pub from: &'a str,
    pub to_name: Option<&'a str>,
    pub to: &'a str,
    pub subject: &'a str,
    pub in_reply_to: Option<&'a str>,
    pub references: &'a [String],
    pub date: DateTime<Local>,
    pub ticket: &'a str,
    pub body: &'a str,
}

/// RFC 5322 message with CRLF line endings, UTF-8 8bit text body.
pub fn build_eml(d: &Draft) -> String {
    let mut h = vec![
        format!("From: {}", mailbox(d.from_name, d.from)),
        format!("To: {}", mailbox(d.to_name, d.to)),
        format!("Subject: {}", header_text(d.subject)),
        format!("Date: {}", d.date.to_rfc2822()),
        format!("Message-ID: <{}>", d.message_id),
    ];
    if let Some(r) = d.in_reply_to {
        h.push(format!("In-Reply-To: <{r}>"));
    }
    if !d.references.is_empty() {
        h.push(format!(
            "References: {}",
            d.references
                .iter()
                .map(|r| format!("<{r}>"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    h.push("MIME-Version: 1.0".into());
    h.push("Content-Type: text/plain; charset=utf-8".into());
    h.push("Content-Transfer-Encoding: 8bit".into());
    h.push(format!("X-Deskcraft-Ticket: {}", d.ticket));
    let body = d.body.replace("\r\n", "\n").replace('\n', "\r\n");
    format!("{}\r\n\r\n{}\r\n", h.join("\r\n"), body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 9)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
    }

    #[test]
    fn parses_mime_encodings_and_threading_headers() {
        let raw = "From: =?utf-8?Q?J=C3=B6rg_Mocky?= <Joerg@Mock.AT>\r\nTo: support@mockdesk.at\r\nSubject: =?utf-8?Q?AW:_Drucker_im_B=C3=BCro_[D-7]?=\r\nDate: Mon, 2 Nov 2026 09:15:00 +0100\r\nMessage-ID: <m2@mock.at>\r\nIn-Reply-To: <s1@mockdesk.at>\r\nReferences: <m1@mock.at> <s1@mockdesk.at>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nLiebe Gr=C3=BC=C3=9Fe aus Wien\r\n";
        let m = parse(raw.as_bytes(), Some("x.eml"), now()).unwrap();
        assert_eq!(m.from_email, "joerg@mock.at");
        assert_eq!(m.from_name.as_deref(), Some("Jörg Mocky"));
        assert_eq!(m.subject, "AW: Drucker im Büro [D-7]");
        assert_eq!(m.body, "Liebe Grüße aus Wien");
        assert_eq!(m.message_id.as_deref(), Some("m2@mock.at"));
        assert_eq!(m.in_reply_to.as_deref(), Some("s1@mockdesk.at"));
        assert_eq!(
            m.references,
            vec!["s1@mockdesk.at", "m1@mock.at"],
            "newest first"
        );
        assert_eq!(m.date.format("%Y-%m-%d").to_string(), "2026-11-02");
        assert_eq!(m.sha256.len(), 64);
        assert!(parse(b"just text", None, now()).is_err());
    }

    #[test]
    fn subjects_and_tags() {
        assert_eq!(normalize_subject("AW: WG: Re:  Drucker [D-12]"), "Drucker");
        assert_eq!(normalize_subject("Fwd: "), "(no subject)");
        assert_eq!(
            normalize_subject("Rechnung [D-x] offen"),
            "Rechnung [D-x] offen"
        );
        assert_eq!(ticket_tag("Re: Drucker [D-12]"), Some(12));
        assert_eq!(ticket_tag("Drucker"), None);
    }

    #[test]
    fn eml_roundtrips_through_the_parser() {
        let refs = vec!["m1@mock.at".to_string(), "m2@mock.at".to_string()];
        let date = Local::now();
        let eml = build_eml(&Draft {
            message_id: "r1@deskcraft.invalid",
            from_name: Some("Support Mock"),
            from: "support@mockdesk.at",
            to_name: Some("Jörg Mocky"),
            to: "joerg@mock.at",
            subject: "Re: Drucker im Büro [D-7]",
            in_reply_to: Some("m2@mock.at"),
            references: &refs,
            date,
            ticket: "D-7",
            body: "Hallo Jörg,\n\nerledigt.",
        });
        assert!(eml.contains("\r\nSubject: =?utf-8?B?"));
        let back = parse(eml.as_bytes(), None, now()).unwrap();
        assert_eq!(back.subject, "Re: Drucker im Büro [D-7]");
        assert_eq!(back.from_email, "support@mockdesk.at");
        assert_eq!(back.message_id.as_deref(), Some("r1@deskcraft.invalid"));
        assert_eq!(back.in_reply_to.as_deref(), Some("m2@mock.at"));
        assert_eq!(back.body, "Hallo Jörg,\n\nerledigt.");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }
}
