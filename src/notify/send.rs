//! Sends one rendered event to one channel. One attempt; the queue decides about retries.

use std::collections::BTreeMap;
use std::time::Duration;

use hmac::{Hmac, Mac};
use lettre::message::{header::ContentType, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use sha2::Sha256;

use super::channel::{ChannelType, NotificationChannel};
use super::render::{self, NotificationEvent};

pub const LINE_PUSH_URL: &str = "https://api.line.me/v2/bot/message/push";
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct SendError {
    pub message: String,
    pub status: Option<u16>,
    /// Worth another attempt (network error, timeout, 408, 429, 5xx, transient SMTP).
    pub retryable: bool,
}

impl SendError {
    fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: None,
            retryable: false,
        }
    }
}

/// `X-CDC-Signature` value: `t=<unix>,v1=<hex HMAC-SHA256(secret, "<t>.<body>")>`.
pub fn signature(secret: &str, unix: i64, body: &[u8]) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(unix.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    format!("t={unix},v1={}", hex::encode(mac.finalize().into_bytes()))
}

fn secret<'a>(secrets: &'a BTreeMap<String, String>, field: &str) -> Result<&'a str, SendError> {
    secrets
        .get(field)
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| SendError::fatal(format!("{field} is not set")))
}

/// Delivers `ev`. Returns the HTTP status (None for SMTP) on success.
pub async fn send(
    http: &reqwest::Client,
    channel: &NotificationChannel,
    secrets: &BTreeMap<String, String>,
    ev: &NotificationEvent,
) -> Result<Option<u16>, SendError> {
    let s = &channel.settings;
    match channel.kind {
        ChannelType::Webhook => {
            let url = s
                .url
                .as_deref()
                .ok_or_else(|| SendError::fatal("URL is not set"))?;
            let body = serde_json::to_vec(&render::webhook_body(ev))
                .map_err(|e| SendError::fatal(e.to_string()))?;
            let mut req = http
                .post(url)
                .header("content-type", "application/json")
                .header("x-cdc-event", ev.condition.as_str());
            if let Some(sec) = secrets.get("signingSecret").filter(|v| !v.is_empty()) {
                req = if s.legacy_secret_header {
                    req.header("x-webhook-secret", sec.as_str())
                } else {
                    req.header(
                        "x-cdc-signature",
                        signature(sec, chrono::Utc::now().timestamp(), &body),
                    )
                };
            }
            send_http(req.body(body)).await
        }
        ChannelType::Slack => {
            post_json(
                http,
                secret(secrets, "webhookUrl")?,
                &render::slack_body(ev),
            )
            .await
        }
        ChannelType::Discord => {
            post_json(
                http,
                secret(secrets, "webhookUrl")?,
                &render::discord_body(ev),
            )
            .await
        }
        ChannelType::Teams => {
            post_json(
                http,
                secret(secrets, "webhookUrl")?,
                &render::teams_body(ev),
            )
            .await
        }
        ChannelType::Line => {
            let to = s
                .line_to
                .as_deref()
                .ok_or_else(|| SendError::fatal("LINE recipient id is not set"))?;
            let token = secret(secrets, "channelAccessToken")?;
            let req = http
                .post(LINE_PUSH_URL)
                .bearer_auth(token)
                .json(&render::line_body(to, ev));
            send_http(req).await
        }
        ChannelType::Email => send_email(channel, secrets, ev).await.map(|_| None),
    }
}

async fn post_json(
    http: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<Option<u16>, SendError> {
    send_http(http.post(url).json(body)).await
}

async fn send_http(req: reqwest::RequestBuilder) -> Result<Option<u16>, SendError> {
    let resp = req.timeout(TIMEOUT).send().await.map_err(|e| SendError {
        // reqwest errors can include the URL; for chat webhooks the URL is the credential.
        message: if e.is_timeout() {
            "timed out".into()
        } else if e.is_connect() {
            "connection failed".into()
        } else {
            "request failed".into()
        },
        status: None,
        retryable: true,
    })?;
    let status = resp.status();
    if status.is_success() {
        return Ok(Some(status.as_u16()));
    }
    let code = status.as_u16();
    let text = resp.text().await.unwrap_or_default();
    let snippet: String = text.trim().chars().take(300).collect();
    Err(SendError {
        message: if snippet.is_empty() {
            format!("HTTP {code}")
        } else {
            format!("HTTP {code}: {snippet}")
        },
        status: Some(code),
        retryable: code == 408 || code == 429 || status.is_server_error(),
    })
}

async fn send_email(
    channel: &NotificationChannel,
    secrets: &BTreeMap<String, String>,
    ev: &NotificationEvent,
) -> Result<(), SendError> {
    let s = &channel.settings;
    let host = s
        .smtp_host
        .as_deref()
        .ok_or_else(|| SendError::fatal("SMTP host is not set"))?;
    let from: Mailbox = s
        .from
        .as_deref()
        .unwrap_or_default()
        .parse()
        .map_err(|_| SendError::fatal("From address is not valid"))?;
    let mut builder = Message::builder()
        .from(from)
        .subject(render::email_subject(ev));
    for to in &s.to {
        builder = builder.to(to
            .parse()
            .map_err(|_| SendError::fatal(format!("recipient is not valid: {to}")))?);
    }
    let message = builder
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(render::plain_text(ev)),
                )
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(render::email_html(ev)),
                ),
        )
        .map_err(|e| SendError::fatal(format!("building message: {e}")))?;

    let security = s.smtp_security.as_deref().unwrap_or("starttls");
    let builder = match security {
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(host),
        "none" => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
            host,
        )),
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host),
    }
    .map_err(|e| SendError::fatal(format!("SMTP setup: {e}")))?;
    let mut builder = builder.timeout(Some(TIMEOUT));
    if let Some(port) = s.smtp_port {
        builder = builder.port(port);
    }
    if let Some(user) = s.smtp_username.as_deref().filter(|u| !u.is_empty()) {
        let pass = secrets.get("smtpPassword").cloned().unwrap_or_default();
        builder = builder.credentials(Credentials::new(user.to_string(), pass));
    }
    builder
        .build()
        .send(message)
        .await
        .map(|_| ())
        .map_err(|e| SendError {
            retryable: !e.is_permanent(),
            status: e.status().map(u16::from),
            message: format!("SMTP: {e}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_a_reference_hmac() {
        // echo -n '1700000000.{"a":1}' | openssl dgst -sha256 -hmac 'k'
        let sig = signature("k", 1_700_000_000, br#"{"a":1}"#);
        assert_eq!(
            sig,
            "t=1700000000,v1=1b6ad1bc9bf48c52ce01939866d5877c6cc4e87ddad27e16673cb8c4964394a3"
        );
        assert_ne!(sig, signature("k2", 1_700_000_000, br#"{"a":1}"#));
        assert_ne!(sig, signature("k", 1_700_000_001, br#"{"a":1}"#));
    }
}
