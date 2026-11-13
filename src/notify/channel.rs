//! Notification channel records (`cdc_config` kind `channel`) and their write-only secrets.
//!
//! Secrets are stored encrypted (`secretbox`) and returned as `********`. On save, `********`
//! or a missing field keeps the stored value, `""` clears it, anything else replaces it.
//! A kept secret may not follow a changed destination (same rule as `saved_secret.rs`).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::saved_secret::REDACTED;
use crate::secretbox::SecretKey;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    Webhook,
    Email,
    Line,
    Slack,
    Discord,
    Teams,
}

impl ChannelType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Webhook => "webhook",
            Self::Email => "email",
            Self::Line => "line",
            Self::Slack => "slack",
            Self::Discord => "discord",
            Self::Teams => "teams",
        }
    }

    /// Secret fields this type accepts, and whether each is required.
    pub fn secret_fields(self) -> &'static [(&'static str, bool)] {
        match self {
            Self::Webhook => &[("signingSecret", false)],
            Self::Email => &[("smtpPassword", false)],
            Self::Line => &[("channelAccessToken", true)],
            Self::Slack | Self::Discord | Self::Teams => &[("webhookUrl", true)],
        }
    }
}

/// Non-secret settings. Flat so one form and one JSON shape serve every type; each type
/// reads only its own fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ChannelSettings {
    // webhook
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Send the secret as `X-Webhook-Secret` (pre-ADR-0015 receivers) instead of signing.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub legacy_secret_header: bool,
    // email
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smtp_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smtp_port: Option<u16>,
    /// `starttls` (default), `tls` (implicit, port 465) or `none`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smtp_security: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smtp_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<String>,
    // line
    /// LINE user, group or room id (`U…`, `C…`, `R…` + 32 hex).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationChannel {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ChannelType,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub settings: ChannelSettings,
    /// Stored: encrypted values. API responses: `********` for each stored field.
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    /// Set by the startup migration of a pre-ADR-0015 rule webhook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrated_from_rule: Option<String>,
}

fn enabled_default() -> bool {
    true
}

impl NotificationChannel {
    /// Copy for API responses: each stored secret becomes `********`.
    pub fn redacted(&self) -> Self {
        let mut c = self.clone();
        for v in c.secrets.values_mut() {
            *v = REDACTED.to_string();
        }
        c
    }

    /// Short, secret-free description of where messages go (for lists and delivery rows).
    pub fn target_label(&self) -> String {
        let s = &self.settings;
        match self.kind {
            ChannelType::Webhook => s.url.as_deref().map(host_of).unwrap_or_default(),
            ChannelType::Email => s.to.join(", "),
            ChannelType::Line => s.line_to.clone().unwrap_or_default(),
            ChannelType::Slack => "Slack incoming webhook".into(),
            ChannelType::Discord => "Discord webhook".into(),
            ChannelType::Teams => "Teams workflow webhook".into(),
        }
    }

    pub fn decrypt_secrets(
        &self,
        key: Option<&SecretKey>,
    ) -> Result<BTreeMap<String, String>, String> {
        if self.secrets.is_empty() {
            return Ok(BTreeMap::new());
        }
        let key = key.ok_or("ORCH_SECRET_KEY is not set; stored secrets cannot be read")?;
        self.secrets
            .iter()
            .map(|(k, v)| key.decrypt(v).map(|p| (k.clone(), p)))
            .collect()
    }
}

pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

fn bad(msg: impl Into<String>) -> AppError {
    AppError::BadRequest(msg.into())
}

fn non_empty(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn check_http_url(field: &str, raw: &str, https_only: bool) -> Result<(), AppError> {
    let u = url::Url::parse(raw.trim()).map_err(|_| bad(format!("{field} is not a valid URL")))?;
    match u.scheme() {
        "https" => Ok(()),
        "http" if !https_only => Ok(()),
        _ => Err(bad(format!(
            "{field} must start with {}",
            if https_only {
                "https://"
            } else {
                "http:// or https://"
            }
        ))),
    }
}

fn is_line_id(s: &str) -> bool {
    s.len() == 33
        && matches!(s.as_bytes()[0], b'U' | b'C' | b'R')
        && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Validates settings and resolves secrets for storage: encrypts new values, keeps stored
/// ones for `********`, and refuses a kept secret whose destination changed.
pub fn prepare_for_store(
    mut incoming: NotificationChannel,
    existing: Option<&NotificationChannel>,
    key: Option<&SecretKey>,
) -> Result<NotificationChannel, AppError> {
    incoming.name = incoming.name.trim().to_string();
    if incoming.name.is_empty() {
        return Err(bad("name is required"));
    }
    validate_settings(&mut incoming)?;

    let existing = existing.filter(|e| e.kind == incoming.kind);
    let mut stored: BTreeMap<String, String> = BTreeMap::new();
    for &(field, required) in incoming.kind.secret_fields() {
        if let Some(v) = resolve_secret(&incoming, existing, key, field)? {
            stored.insert(field.to_string(), v);
        } else if required {
            return Err(bad(format!(
                "{} is required for a {} channel",
                label(field),
                incoming.kind.as_str()
            )));
        }
    }
    incoming.secrets = stored;
    let now = Utc::now();
    incoming.created_at = existing.and_then(|e| e.created_at).or(Some(now));
    incoming.updated_at = Some(now);
    if incoming.migrated_from_rule.is_none() {
        incoming.migrated_from_rule = existing.and_then(|e| e.migrated_from_rule.clone());
    }
    Ok(incoming)
}

/// Stored (encrypted) value for one secret field: a new value is encrypted, `********` or
/// an absent value keeps the stored one, and an empty string clears it.
fn resolve_secret(
    incoming: &NotificationChannel,
    existing: Option<&NotificationChannel>,
    key: Option<&SecretKey>,
    field: &str,
) -> Result<Option<String>, AppError> {
    match incoming.secrets.get(field).map(String::as_str) {
        None | Some(REDACTED) => {
            let Some((old, prev)) = existing.and_then(|e| e.secrets.get(field).map(|p| (e, p)))
            else {
                return Ok(None);
            };
            if let Some(why) = destination_changed(incoming, old) {
                return Err(bad(format!("{why}; enter the {} again", label(field))));
            }
            Ok(Some(prev.clone()))
        }
        Some("") => Ok(None),
        Some(plain) => {
            if incoming.kind != ChannelType::Webhook && field == "webhookUrl" {
                check_http_url(label(field), plain, true)?;
            }
            let key = key.ok_or_else(|| {
                bad("ORCH_SECRET_KEY is not set, so channel secrets cannot be stored. \
                     Set it to 32 random bytes (`openssl rand -base64 32`) and restart the orchestrator")
            })?;
            Ok(Some(key.encrypt(plain.trim())))
        }
    }
}

fn label(field: &str) -> &'static str {
    match field {
        "signingSecret" => "signing secret",
        "smtpPassword" => "SMTP password",
        "channelAccessToken" => "channel access token",
        "webhookUrl" => "webhook URL",
        _ => "secret",
    }
}

/// Why a stored secret may not be reused for the edited channel, if it may not.
fn destination_changed(new: &NotificationChannel, old: &NotificationChannel) -> Option<String> {
    let (n, o) = (&new.settings, &old.settings);
    match new.kind {
        ChannelType::Email => {
            let host = |s: &ChannelSettings| {
                s.smtp_host
                    .clone()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase()
            };
            let user = |s: &ChannelSettings| {
                s.smtp_username
                    .clone()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            };
            (host(n) != host(o) || user(n) != user(o))
                .then(|| "SMTP host or username changed".to_string())
        }
        // The legacy header sends the secret itself to the URL.
        ChannelType::Webhook if n.legacy_secret_header => {
            let h = |s: &ChannelSettings| s.url.as_deref().map(host_of).unwrap_or_default();
            (h(n) != h(o)).then(|| "webhook host changed".to_string())
        }
        _ => None,
    }
}

fn validate_settings(c: &mut NotificationChannel) -> Result<(), AppError> {
    let s = &mut c.settings;
    match c.kind {
        ChannelType::Webhook => {
            let url =
                non_empty(&s.url).ok_or_else(|| bad("URL is required for a webhook channel"))?;
            check_http_url("URL", url, false)?;
            s.url = Some(url.to_string());
        }
        ChannelType::Email => {
            let host = non_empty(&s.smtp_host)
                .ok_or_else(|| bad("SMTP host is required"))?
                .to_string();
            let security = s
                .smtp_security
                .as_deref()
                .unwrap_or("starttls")
                .trim()
                .to_ascii_lowercase();
            if !matches!(security.as_str(), "starttls" | "tls" | "none") {
                return Err(bad("SMTP security must be starttls, tls or none"));
            }
            let from = non_empty(&s.from)
                .ok_or_else(|| bad("From address is required"))?
                .to_string();
            from.parse::<lettre::message::Mailbox>()
                .map_err(|_| bad(format!("From address is not valid: {from}")))?;
            let to: Vec<String> =
                s.to.iter()
                    .flat_map(|v| v.split([',', ';', '\n']))
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                    .collect();
            if to.is_empty() {
                return Err(bad("At least one recipient is required"));
            }
            for addr in &to {
                addr.parse::<lettre::message::Mailbox>()
                    .map_err(|_| bad(format!("Recipient is not a valid address: {addr}")))?;
            }
            s.smtp_port = Some(
                s.smtp_port
                    .filter(|p| *p > 0)
                    .unwrap_or(match security.as_str() {
                        "tls" => 465,
                        "none" => 25,
                        _ => 587,
                    }),
            );
            s.smtp_host = Some(host);
            s.smtp_security = Some(security);
            s.smtp_username = non_empty(&s.smtp_username).map(str::to_string);
            s.from = Some(from);
            s.to = to;
        }
        ChannelType::Line => {
            let to = non_empty(&s.line_to)
                .ok_or_else(|| bad("LINE recipient id (user, group or room) is required"))?
                .to_string();
            if !is_line_id(&to) {
                return Err(bad(
                    "LINE recipient id must be U, C or R followed by 32 hex characters (a user, group or room id from the Messaging API)",
                ));
            }
            s.line_to = Some(to);
        }
        ChannelType::Slack | ChannelType::Discord | ChannelType::Teams => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SecretKey {
        SecretKey::parse(&"11".repeat(32)).unwrap()
    }

    fn email(host: &str, user: &str, password: Option<&str>) -> NotificationChannel {
        let mut secrets = BTreeMap::new();
        if let Some(p) = password {
            secrets.insert("smtpPassword".to_string(), p.to_string());
        }
        NotificationChannel {
            id: "c1".into(),
            name: "Ops mail".into(),
            kind: ChannelType::Email,
            enabled: true,
            settings: ChannelSettings {
                smtp_host: Some(host.into()),
                smtp_username: Some(user.into()),
                from: Some("cdc@example.com".into()),
                to: vec!["a@example.com, b@example.com".into()],
                ..Default::default()
            },
            secrets,
            created_at: None,
            updated_at: None,
            migrated_from_rule: None,
        }
    }

    #[test]
    fn secrets_are_encrypted_redacted_in_responses_and_kept_on_redacted_resend() {
        let k = key();
        let saved = prepare_for_store(email("smtp.example.com", "ops", Some("pw")), None, Some(&k))
            .unwrap();
        let stored = saved.secrets.get("smtpPassword").unwrap();
        assert!(stored.starts_with("v1:"));
        assert_eq!(saved.redacted().secrets["smtpPassword"], REDACTED);
        assert_eq!(saved.settings.to, vec!["a@example.com", "b@example.com"]);
        assert_eq!(saved.settings.smtp_port, Some(587));

        let again = prepare_for_store(
            email("smtp.example.com", "ops", Some(REDACTED)),
            Some(&saved),
            Some(&k),
        )
        .unwrap();
        assert_eq!(
            again.secrets["smtpPassword"], *stored,
            "redacted resend keeps the stored value"
        );
        assert_eq!(
            again.decrypt_secrets(Some(&k)).unwrap()["smtpPassword"],
            "pw"
        );

        let cleared = prepare_for_store(
            email("smtp.example.com", "ops", Some("")),
            Some(&saved),
            Some(&k),
        )
        .unwrap();
        assert!(cleared.secrets.is_empty());
    }

    #[test]
    fn a_kept_password_may_not_follow_a_changed_smtp_host() {
        let k = key();
        let saved = prepare_for_store(email("smtp.example.com", "ops", Some("pw")), None, Some(&k))
            .unwrap();
        let err = prepare_for_store(
            email("evil.example.net", "ops", Some(REDACTED)),
            Some(&saved),
            Some(&k),
        )
        .unwrap_err();
        assert!(err.to_string().contains("SMTP host or username changed"));
        // Entering it again is fine.
        assert!(prepare_for_store(
            email("evil.example.net", "ops", Some("pw2")),
            Some(&saved),
            Some(&k)
        )
        .is_ok());
    }

    #[test]
    fn secrets_need_the_key_and_required_secrets_must_be_present() {
        let err = prepare_for_store(email("smtp.example.com", "ops", Some("pw")), None, None)
            .unwrap_err();
        assert!(err.to_string().contains("ORCH_SECRET_KEY"));
        // No password at all is fine without a key (unauthenticated relay).
        assert!(prepare_for_store(email("smtp.example.com", "", None), None, None).is_ok());

        let slack = NotificationChannel {
            kind: ChannelType::Slack,
            secrets: BTreeMap::new(),
            ..email("x", "y", None)
        };
        assert!(prepare_for_store(slack, None, Some(&key()))
            .unwrap_err()
            .to_string()
            .contains("webhook URL is required"));
    }

    #[test]
    fn line_ids_and_urls_are_checked() {
        let mut line = NotificationChannel {
            kind: ChannelType::Line,
            settings: ChannelSettings {
                line_to: Some("not-an-id".into()),
                ..Default::default()
            },
            secrets: BTreeMap::from([("channelAccessToken".to_string(), "tok".to_string())]),
            ..email("x", "y", None)
        };
        assert!(prepare_for_store(line.clone(), None, Some(&key())).is_err());
        line.settings.line_to = Some(format!("U{}", "a".repeat(32)));
        assert!(prepare_for_store(line, None, Some(&key())).is_ok());

        let discord = NotificationChannel {
            kind: ChannelType::Discord,
            settings: ChannelSettings::default(),
            secrets: BTreeMap::from([("webhookUrl".to_string(), "http://insecure".to_string())]),
            ..email("x", "y", None)
        };
        assert!(prepare_for_store(discord, None, Some(&key()))
            .unwrap_err()
            .to_string()
            .contains("https://"));
    }

    #[test]
    fn unknown_secret_fields_are_dropped() {
        let mut c = email("smtp.example.com", "ops", Some("pw"));
        c.secrets.insert("other".into(), "x".into());
        let saved = prepare_for_store(c, None, Some(&key())).unwrap();
        assert_eq!(
            saved.secrets.keys().collect::<Vec<_>>(),
            vec!["smtpPassword"]
        );
    }
}
