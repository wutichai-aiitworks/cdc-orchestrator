//! One alert event, rendered in each channel's native format (pure; no I/O).

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    Warning,
    Info,
    Resolved,
}

impl Severity {
    fn marker(self) -> &'static str {
        match self {
            Self::Critical => "🔴",
            Self::Warning => "🟠",
            Self::Info => "🔵",
            Self::Resolved => "✅",
        }
    }
    /// Discord embed colour.
    fn rgb(self) -> u32 {
        match self {
            Self::Critical => 0xDC2626,
            Self::Warning => 0xF59E0B,
            Self::Info => 0x3B82F6,
            Self::Resolved => 0x16A34A,
        }
    }
    /// Adaptive Card text colour.
    fn card_color(self) -> &'static str {
        match self {
            Self::Critical => "Attention",
            Self::Warning => "Warning",
            Self::Info => "Accent",
            Self::Resolved => "Good",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationEvent {
    /// Rule condition that produced it (`connector_failed`, `pipeline_failed`, `test`, …).
    pub condition: String,
    pub severity: Severity,
    pub title: String,
    pub summary: String,
    /// Connector, pipeline or topic name.
    pub subject: String,
    /// `connector`, `pipeline` or `topic`.
    pub subject_kind: String,
    pub from: Option<String>,
    pub to: String,
    pub errors: Vec<String>,
    pub ts: DateTime<Utc>,
    pub rule: Option<String>,
    pub link: Option<String>,
    pub test: bool,
}

impl NotificationEvent {
    pub fn sample() -> Self {
        Self {
            condition: "test".into(),
            severity: Severity::Info,
            title: "Test notification from CDC Orchestrator".into(),
            summary: "If you can read this, the channel works.".into(),
            subject: "example-connector".into(),
            subject_kind: "connector".into(),
            from: Some("RUNNING".into()),
            to: "FAILED".into(),
            errors: vec![
                "Task 0: org.apache.kafka.connect.errors.ConnectException: sample error".into(),
            ],
            ts: Utc::now(),
            rule: Some("Test".into()),
            link: None,
            test: true,
        }
    }
}

const MAX_ERRORS: usize = 3;
const MAX_ERROR_CHARS: usize = 300;

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// First line of each error (stack traces stay in the logs), clipped, at most three.
pub fn error_lines(ev: &NotificationEvent) -> Vec<String> {
    let mut out: Vec<String> = ev
        .errors
        .iter()
        .take(MAX_ERRORS)
        .map(|e| clip(e.lines().next().unwrap_or("").trim(), MAX_ERROR_CHARS))
        .collect();
    if ev.errors.len() > MAX_ERRORS {
        out.push(format!("… and {} more", ev.errors.len() - MAX_ERRORS));
    }
    out
}

fn time_label(ev: &NotificationEvent) -> String {
    ev.ts.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// Plain text: LINE, email text part, and Slack's notification fallback.
pub fn plain_text(ev: &NotificationEvent) -> String {
    let mut s = format!(
        "{} {}\n{}\n{}",
        ev.severity.marker(),
        ev.title,
        ev.summary,
        time_label(ev)
    );
    if let Some(rule) = &ev.rule {
        s.push_str(&format!("\nRule: {rule}"));
    }
    let errs = error_lines(ev);
    if !errs.is_empty() {
        s.push_str("\n\nErrors:");
        for e in errs {
            s.push_str(&format!("\n• {e}"));
        }
    }
    if let Some(link) = &ev.link {
        s.push_str(&format!("\n\nOpen: {link}"));
    }
    s
}

/// Generic webhook JSON: the pre-ADR-0015 fields first, so existing receivers keep working.
pub fn webhook_body(ev: &NotificationEvent) -> Value {
    json!({
        "event": ev.condition,
        "connector": ev.subject,
        "from": ev.from,
        "to": ev.to,
        "ts": ev.ts,
        "errors": ev.errors,
        "severity": ev.severity,
        "title": ev.title,
        "summary": ev.summary,
        "subject": ev.subject,
        "subjectKind": ev.subject_kind,
        "rule": ev.rule,
        "link": ev.link,
        "test": ev.test,
    })
}

fn slack_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn slack_body(ev: &NotificationEvent) -> Value {
    let mut blocks = vec![json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": format!("{} *{}*\n{}", ev.severity.marker(), slack_escape(&ev.title), slack_escape(&ev.summary))}
    })];
    let errs = error_lines(ev);
    if !errs.is_empty() {
        blocks.push(json!({
            "type": "section",
            "text": {"type": "mrkdwn", "text": format!("```{}```", slack_escape(&errs.join("\n")))}
        }));
    }
    let mut context = vec![json!({"type": "mrkdwn", "text": time_label(ev)})];
    if let Some(rule) = &ev.rule {
        context.push(json!({"type": "mrkdwn", "text": format!("Rule: {}", slack_escape(rule))}));
    }
    if let Some(link) = &ev.link {
        context.push(json!({"type": "mrkdwn", "text": format!("<{link}|Open in CDC>")}));
    }
    blocks.push(json!({"type": "context", "elements": context}));
    json!({"text": clip(&plain_text(ev), 3000), "blocks": blocks})
}

pub fn discord_body(ev: &NotificationEvent) -> Value {
    let mut description = ev.summary.clone();
    let errs = error_lines(ev);
    if !errs.is_empty() {
        description.push_str(&format!("\n```\n{}\n```", errs.join("\n")));
    }
    let mut embed = json!({
        "title": clip(&format!("{} {}", ev.severity.marker(), ev.title), 256),
        "description": clip(&description, 4000),
        "color": ev.severity.rgb(),
        "timestamp": ev.ts,
    });
    if let Some(rule) = &ev.rule {
        embed["footer"] = json!({"text": clip(&format!("Rule: {rule}"), 2000)});
    }
    if let Some(link) = &ev.link {
        embed["url"] = json!(link);
    }
    json!({"username": "CDC Orchestrator", "embeds": [embed]})
}

/// Teams Workflows webhook: a message with one Adaptive Card.
pub fn teams_body(ev: &NotificationEvent) -> Value {
    let mut facts = vec![
        json!({"title": ev.subject_kind.clone(), "value": ev.subject}),
        json!({"title": "Time", "value": time_label(ev)}),
    ];
    if let Some(rule) = &ev.rule {
        facts.push(json!({"title": "Rule", "value": rule}));
    }
    let mut body = vec![
        json!({"type": "TextBlock", "text": ev.title, "weight": "Bolder", "size": "Medium", "wrap": true, "color": ev.severity.card_color()}),
        json!({"type": "TextBlock", "text": ev.summary, "wrap": true}),
        json!({"type": "FactSet", "facts": facts}),
    ];
    let errs = error_lines(ev);
    if !errs.is_empty() {
        body.push(json!({"type": "TextBlock", "text": errs.join("\n\n"), "wrap": true, "fontType": "Monospace", "size": "Small"}));
    }
    let mut card = json!({
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "type": "AdaptiveCard",
        "version": "1.4",
        "body": body,
    });
    if let Some(link) = &ev.link {
        card["actions"] = json!([{"type": "Action.OpenUrl", "title": "Open in CDC", "url": link}]);
    }
    json!({
        "type": "message",
        "attachments": [{"contentType": "application/vnd.microsoft.card.adaptive", "contentUrl": null, "content": card}]
    })
}

/// LINE Messaging API push body. Text messages are limited to 5000 characters.
pub fn line_body(to: &str, ev: &NotificationEvent) -> Value {
    json!({"to": to, "messages": [{"type": "text", "text": clip(&plain_text(ev), 5000)}]})
}

pub fn email_subject(ev: &NotificationEvent) -> String {
    let tag = match ev.severity {
        Severity::Critical => "[CDC] ",
        Severity::Warning => "[CDC warning] ",
        Severity::Info => "[CDC] ",
        Severity::Resolved => "[CDC resolved] ",
    };
    clip(&format!("{tag}{}", ev.title), 200)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn email_html(ev: &NotificationEvent) -> String {
    let color = format!("#{:06x}", ev.severity.rgb());
    let mut rows = format!(
        "<tr><td style=\"color:#666;padding:2px 12px 2px 0\">{}</td><td>{}</td></tr>\
         <tr><td style=\"color:#666;padding:2px 12px 2px 0\">Time</td><td>{}</td></tr>",
        html_escape(&ev.subject_kind),
        html_escape(&ev.subject),
        time_label(ev)
    );
    if let Some(rule) = &ev.rule {
        rows.push_str(&format!(
            "<tr><td style=\"color:#666;padding:2px 12px 2px 0\">Rule</td><td>{}</td></tr>",
            html_escape(rule)
        ));
    }
    let errs = error_lines(ev);
    let errors = if errs.is_empty() {
        String::new()
    } else {
        format!(
            "<pre style=\"background:#f6f6f6;padding:10px;border-radius:6px;white-space:pre-wrap;font-size:12px\">{}</pre>",
            html_escape(&errs.join("\n"))
        )
    };
    let link = ev
        .link
        .as_ref()
        .map(|l| format!("<p><a href=\"{0}\">Open in CDC</a></p>", html_escape(l)))
        .unwrap_or_default();
    format!(
        "<div style=\"font-family:-apple-system,Segoe UI,sans-serif;font-size:14px;color:#111\">\
         <div style=\"border-left:4px solid {color};padding-left:12px\">\
         <h2 style=\"margin:0 0 4px;font-size:16px\">{}</h2><p style=\"margin:0 0 12px\">{}</p></div>\
         <table style=\"font-size:13px;margin:8px 0\">{rows}</table>{errors}{link}\
         <p style=\"color:#999;font-size:12px\">Sent by hyper-cdc-orchestrator</p></div>",
        html_escape(&ev.title),
        html_escape(&ev.summary),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failing() -> NotificationEvent {
        NotificationEvent {
            condition: "connector_failed".into(),
            severity: Severity::Critical,
            title: "Connector failed: orders-source".into(),
            summary: "RUNNING → FAILED".into(),
            subject: "orders-source".into(),
            subject_kind: "connector".into(),
            from: Some("RUNNING".into()),
            to: "FAILED".into(),
            errors: (0..5)
                .map(|i| format!("Task {i}: boom <x>\n\tat a.b.C(C.java:1)"))
                .collect(),
            ts: "2026-11-12T14:44:51Z".parse().unwrap(),
            rule: Some("Prod failures".into()),
            link: Some("https://cdc.example.com/connectors".into()),
            test: false,
        }
    }

    #[test]
    fn webhook_keeps_the_legacy_fields() {
        let v = webhook_body(&failing());
        assert_eq!(v["event"], "connector_failed");
        assert_eq!(v["connector"], "orders-source");
        assert_eq!(v["from"], "RUNNING");
        assert_eq!(v["to"], "FAILED");
        assert_eq!(v["errors"].as_array().unwrap().len(), 5);
        assert_eq!(v["severity"], "critical");
    }

    #[test]
    fn errors_are_first_lines_capped_at_three() {
        let lines = error_lines(&failing());
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "Task 0: boom <x>");
        assert_eq!(lines[3], "… and 2 more");
    }

    #[test]
    fn text_formats_include_title_rule_and_link() {
        let t = plain_text(&failing());
        assert!(t.starts_with(
            "🔴 Connector failed: orders-source\nRUNNING → FAILED\n2026-11-12 14:44:51 UTC"
        ));
        assert!(
            t.contains("Rule: Prod failures")
                && t.contains("Open: https://cdc.example.com/connectors")
        );
        let line = line_body(&format!("U{}", "0".repeat(32)), &failing());
        assert_eq!(line["messages"][0]["type"], "text");
        assert_eq!(
            email_subject(&failing()),
            "[CDC] Connector failed: orders-source"
        );
    }

    #[test]
    fn markup_is_escaped_per_target() {
        let s = slack_body(&failing()).to_string();
        assert!(s.contains("boom &lt;x&gt;"));
        let h = email_html(&failing());
        assert!(h.contains("boom &lt;x&gt;") && !h.contains("boom <x>"));
    }

    #[test]
    fn chat_payloads_have_the_shapes_each_service_expects() {
        let d = discord_body(&failing());
        assert_eq!(d["embeds"][0]["color"], 0xDC2626);
        let t = teams_body(&failing());
        assert_eq!(
            t["attachments"][0]["contentType"],
            "application/vnd.microsoft.card.adaptive"
        );
        assert_eq!(
            t["attachments"][0]["content"]["actions"][0]["type"],
            "Action.OpenUrl"
        );
    }

    #[test]
    fn line_text_is_clipped_to_the_api_limit() {
        let mut ev = failing();
        ev.summary = "x".repeat(6000);
        let body = line_body("U0", &ev);
        assert!(
            body["messages"][0]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= 5000
        );
    }
}
