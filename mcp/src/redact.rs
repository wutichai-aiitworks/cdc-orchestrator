//! Secret masking for everything mcp-cdc sends back to a client. MCP output lands in model
//! context and chat transcripts, and several orchestrator answers carry secrets as-is (pgcopy
//! create/update returns the saved passwords; Kafka Connect configs hold `database.password`;
//! logs can contain conninfo strings). One pass over every tool result and error message.

use serde_json::Value;

pub(crate) const MASK: &str = "********";

/// Keys whose string value is always a secret.
fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.contains("password")
        || k.contains("passwd")
        || k.contains("secret")
        || k.contains("passphrase")
        || k.contains("private_key")
        || k.contains("privatekey")
        || k.contains("api_key")
        || k.contains("apikey")
        || k.contains("jaas")
        || k.ends_with("token")
        || k.ends_with("credentials")
        || k == "authorization"
}

pub(crate) fn redact(mut v: Value) -> Value {
    walk(&mut v);
    v
}

fn walk(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                match val {
                    Value::String(s) if is_secret_key(k) => {
                        if !s.is_empty() {
                            *s = MASK.to_string();
                        }
                    }
                    _ => walk(val),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(walk),
        Value::String(s) => {
            let masked = redact_text(s);
            if masked != *s {
                *s = masked;
            }
        }
        _ => {}
    }
}

/// `key=value` / `"key":"value"` / `key: value` forms that appear in logs, conninfo strings
/// and error bodies. Lowercase needles; matching is ASCII case-insensitive.
const NEEDLES: &[&str] = &[
    "password=",
    "password\":\"",
    "password\": \"",
    "password: ",
    "passwd=",
    "pwd=",
    "secret=",
    "secret\":\"",
    "token=",
    "token\":\"",
    "apikey=",
    "api_key=",
];

const STOP: &[char] = &[
    ' ', '\t', '\n', '\r', '&', ';', '"', '\'', ',', '}', ')', ']',
];

/// Mask secrets inside free text: `password=…` style pairs and the password in
/// `scheme://user:password@host` URIs.
pub(crate) fn redact_text(s: &str) -> String {
    // ASCII lowercasing keeps byte offsets identical, so ranges found in `lower` apply to `s`.
    let lower = s.to_ascii_lowercase();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for needle in NEEDLES {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(needle) {
            let start = from + pos + needle.len();
            let end = s[start..].find(STOP).map_or(s.len(), |i| start + i);
            if end > start && &s[start..end] != MASK {
                ranges.push((start, end));
            }
            from = start.max(from + pos + 1);
        }
    }
    let mut from = 0;
    while let Some(pos) = s[from..].find("://") {
        let auth_start = from + pos + 3;
        let rest = &s[auth_start..];
        let auth_end = rest
            .find(|c: char| c == '/' || c.is_whitespace())
            .unwrap_or(rest.len());
        if let Some(at) = rest[..auth_end].rfind('@') {
            if let Some(colon) = rest[..at].find(':') {
                let (a, b) = (auth_start + colon + 1, auth_start + at);
                if b > a && &s[a..b] != MASK {
                    ranges.push((a, b));
                }
            }
        }
        from = auth_start;
    }
    if ranges.is_empty() {
        return s.to_string();
    }
    ranges.sort_unstable();
    let mut out = String::with_capacity(s.len());
    let mut cursor = 0;
    for (a, b) in ranges {
        if a < cursor {
            continue; // overlaps a range already masked
        }
        out.push_str(&s[cursor..a]);
        out.push_str(MASK);
        cursor = b;
    }
    out.push_str(&s[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn masks_secret_keys_at_any_depth() {
        let v = redact(json!({
            "source": {"password": "x", "host": "h"},
            "config": {"database.password": "p", "sasl.jaas.config": "j", "connection.user": "u"},
            "uploads": [{"ssh_password": "y", "name": "n"}],
            "empty": {"password": ""},
            "opts": {"no_role_passwords": true},
            "token_env": "ORCH_PGCOPY_TOKEN"
        }));
        assert_eq!(v["source"]["password"], MASK);
        assert_eq!(v["source"]["host"], "h");
        assert_eq!(v["config"]["database.password"], MASK);
        assert_eq!(v["config"]["sasl.jaas.config"], MASK);
        assert_eq!(v["config"]["connection.user"], "u");
        assert_eq!(v["uploads"][0]["ssh_password"], MASK);
        assert_eq!(v["empty"]["password"], "");
        assert_eq!(v["opts"]["no_role_passwords"], true);
        assert_eq!(v["token_env"], "ORCH_PGCOPY_TOKEN");
    }

    #[test]
    fn masks_secrets_in_free_text() {
        assert_eq!(
            redact_text("host=db password=s3cr3t user=u"),
            "host=db password=******** user=u"
        );
        assert_eq!(redact_text("PASSWORD=abc&x=1"), "PASSWORD=********&x=1");
        assert_eq!(
            redact_text(r#"{"password":"abc"}"#),
            r#"{"password":"********"}"#
        );
        assert_eq!(
            redact_text("postgres://user:p%40ss@db:5432/app and mongodb://u:pw@m/x"),
            "postgres://user:********@db:5432/app and mongodb://u:********@m/x"
        );
        assert_eq!(
            redact_text("https://example.com/path"),
            "https://example.com/path"
        );
        assert_eq!(redact_text("http://user@host/"), "http://user@host/");
        assert_eq!(redact_text("password=********"), "password=********");
        assert_eq!(
            redact_text("ทดสอบ password=ค่า ok"),
            "ทดสอบ password=******** ok"
        );
    }

    #[test]
    fn walks_strings_inside_arrays() {
        let v = redact(json!({"lines": ["ok", "conn password=x failed"]}));
        assert_eq!(v["lines"][1], "conn password=******** failed");
    }
}
