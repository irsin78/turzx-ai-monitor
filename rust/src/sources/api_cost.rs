//! Gemini API spend this month, from a Google Cloud budget: the budget publishes its accrued
//! cost to a Pub/Sub topic several times a day, and the dashboard pulls the messages from a
//! subscription with a service account that may only read that subscription. There is no API
//! that answers "how much did this API key cost".

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ai::{agent, local_appdata, now};

/// What the budget reported last.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ApiCost {
    pub amount: f64,
    /// ISO code of the billing account ("KRW", "USD")
    pub currency: String,
    /// start of the budget period (epoch seconds); a monthly budget starts over after a month
    pub interval_start: f64,
    /// when the notification was published (epoch seconds)
    pub published: f64,
}

impl ApiCost {
    /// The reported amount while its period lasts, 0 once a new month began (the first
    /// notification of the new period comes within hours).
    pub fn current(&self) -> f64 {
        let ended = chrono::DateTime::from_timestamp(self.interval_start as i64, 0)
            .and_then(|t| t.checked_add_months(chrono::Months::new(1)))
            .is_some_and(|end| end.timestamp() as f64 <= now());
        if ended { 0.0 } else { self.amount }
    }
}

/// The subscription from the settings, or None when the feature is off.
pub fn subscription() -> Option<String> {
    let sub = crate::config::get().api_cost.subscription.trim();
    (!sub.is_empty()).then(|| sub.to_string())
}

/// The service account key file (looked for on every read: it may be put there after the
/// settings were saved, or removed).
pub fn key_file() -> PathBuf {
    let k = crate::config::get().api_cost.key_file.trim();
    if k.is_empty() { crate::config::dir().join("gcp-key.json") } else { PathBuf::from(k) }
}

/// No service account key: the line goes away until one is there.
#[derive(Debug)]
pub struct NoKey;
impl std::fmt::Display for NoKey {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "no service account key file")
    }
}
impl std::error::Error for NoKey {}

fn cache_path() -> PathBuf {
    local_appdata().join(r"TurzxDashboard\api_cost.json")
}

fn read_cache() -> Value {
    std::fs::read(cache_path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

/// The last value and when it was fetched.
pub fn cached() -> (Option<ApiCost>, f64) {
    let v = read_cache();
    (serde_json::from_value(v["cost"].clone()).ok(), v["fetched_at"].as_f64().unwrap_or(0.0))
}

fn store(c: Option<&ApiCost>) {
    let mut v = read_cache();
    if let Some(c) = c {
        v["cost"] = serde_json::to_value(c).unwrap();
    }
    v["fetched_at"] = json!(now());
    let path = cache_path();
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec(&v).unwrap()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Service account key -> OAuth access token (JWT bearer grant, RS256).
fn access_token(key_file: &PathBuf) -> Result<String> {
    let key: Value = serde_json::from_slice(&std::fs::read(key_file).context("service account key")?).context("service account key: not JSON")?;
    let field = |k: &str| key[k].as_str().with_context(|| format!("service account key: no {k}"));
    let token_uri = key["token_uri"].as_str().unwrap_or("https://oauth2.googleapis.com/token");
    let iat = now() as i64;
    let header = json!({"alg": "RS256", "typ": "JWT", "kid": field("private_key_id")?});
    let claims = json!({
        "iss": field("client_email")?,
        "scope": "https://www.googleapis.com/auth/pubsub",
        "aud": token_uri,
        "iat": iat,
        "exp": iat + 3600,
    });
    let unsigned = format!("{}.{}", URL_SAFE_NO_PAD.encode(header.to_string()), URL_SAFE_NO_PAD.encode(claims.to_string()));
    let pem = field("private_key")?;
    let der = STANDARD.decode(pem.lines().filter(|l| !l.starts_with("-----")).collect::<String>().trim()).context("service account key: bad private_key")?;
    let pair = ring::signature::RsaKeyPair::from_pkcs8(&der).map_err(|e| anyhow::anyhow!("service account key: {e}"))?;
    let mut sig = vec![0u8; pair.public().modulus_len()];
    pair.sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), unsigned.as_bytes(), &mut sig)
        .map_err(|_| anyhow::anyhow!("could not sign the token request"))?;
    let jwt = format!("{unsigned}.{}", URL_SAFE_NO_PAD.encode(sig));
    let mut r = agent()
        .post(token_uri)
        .send_form([("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", jwt.as_str())])?;
    let status = r.status().as_u16();
    let body: Value = r.body_mut().read_json().unwrap_or(Value::Null);
    if status != 200 {
        bail!("token request: HTTP {status} {}", body["error"].as_str().unwrap_or(""));
    }
    Ok(body["access_token"].as_str().context("token request: no access_token")?.to_string())
}

fn post(token: &str, url: &str, body: Value) -> Result<Value> {
    let mut r = agent().post(url).header("Authorization", format!("Bearer {token}")).send_json(&body)?;
    let status = r.status().as_u16();
    let reply: Value = r.body_mut().read_json().unwrap_or(Value::Null);
    if status != 200 {
        bail!("Pub/Sub: HTTP {status} {}", reply["error"]["status"].as_str().unwrap_or(""));
    }
    Ok(reply)
}

/// One budget notification (the message data).
fn parse(data_b64: &str, publish_time: &str) -> Option<ApiCost> {
    let data: Value = serde_json::from_slice(&STANDARD.decode(data_b64).ok()?).ok()?;
    let ts = |s: &str| chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp_millis() as f64 / 1000.0);
    Some(ApiCost {
        amount: data["costAmount"].as_f64()?,
        currency: data["currencyCode"].as_str().unwrap_or("").to_string(),
        interval_start: data["costIntervalStart"].as_str().and_then(ts).unwrap_or(0.0),
        published: ts(publish_time).unwrap_or(0.0),
    })
}

/// Take every waiting notification off the subscription and return the newest report known:
/// the newest message (latest period, then latest publish time) or the cached one, whichever
/// is newer (messages come out of order, and `diag api-cost` may have taken them). Pub/Sub
/// may return nothing even when messages are waiting; they come with the next read.
pub fn fetch() -> Result<Option<ApiCost>> {
    let Some(sub) = subscription() else { bail!("not set up") };
    let key = key_file();
    if !key.is_file() {
        return Err(NoKey.into());
    }
    let token = access_token(&key)?;
    let base = format!("https://pubsub.googleapis.com/v1/{sub}");
    let newer = |c: &ApiCost, than: &Option<ApiCost>| than.as_ref().is_none_or(|n| (c.interval_start, c.published) > (n.interval_start, n.published));
    let (mut newest, _) = cached();
    let mut failed = None;
    for round in 0..10 {
        let reply = match post(&token, &format!("{base}:pull"), json!({"maxMessages": 100, "returnImmediately": true})) {
            Ok(r) => r,
            Err(e) => {
                failed = Some(e);
                break;
            }
        };
        let msgs = reply["receivedMessages"].as_array().cloned().unwrap_or_default();
        if msgs.is_empty() {
            break;
        }
        let mut acks = Vec::new();
        for m in &msgs {
            if let Some(id) = m["ackId"].as_str() {
                acks.push(id.to_string());
            }
            let msg = &m["message"];
            if let Some(c) = parse(msg["data"].as_str().unwrap_or(""), msg["publishTime"].as_str().unwrap_or("")) {
                if newer(&c, &newest) {
                    newest = Some(c);
                }
            }
        }
        // keep what was read before acknowledging: once acked, Pub/Sub does not send it again
        store(newest.as_ref());
        if let Err(e) = post(&token, &format!("{base}:acknowledge"), json!({"ackIds": acks})) {
            log::warn!("Gemini API cost: acknowledge failed after round {round}: {e:#}");
            break;
        }
    }
    match failed {
        // nothing was read this time: say why (what was read before is kept and shown)
        Some(e) if newest.is_none() => Err(e),
        Some(e) => {
            log::warn!("Gemini API cost: {e:#}");
            Ok(newest)
        }
        None => {
            store(newest.as_ref());
            Ok(newest)
        }
    }
}

/// "₩12,345", "$1.23", "12.34 EUR".
pub fn format(amount: f64, currency: &str) -> String {
    let (symbol, decimals) = match currency {
        "KRW" => ("₩", 0),
        "JPY" => ("¥", 0),
        "USD" => ("$", 2),
        "EUR" => ("€", 2),
        "GBP" => ("£", 2),
        _ => ("", 2),
    };
    let fixed = format!("{:.decimals$}", amount.abs());
    let sign = if amount < 0.0 && fixed.chars().any(|c| c.is_ascii_digit() && c != '0') { "-" } else { "" };
    let (int, frac) = fixed.split_once('.').map_or((fixed.as_str(), None), |(i, f)| (i, Some(f)));
    let mut grouped = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let num = frac.map_or(grouped.clone(), |f| format!("{grouped}.{f}"));
    if symbol.is_empty() { format!("{sign}{num} {currency}").trim().to_string() } else { format!("{sign}{symbol}{num}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_by_currency() {
        assert_eq!(format(12345.4, "KRW"), "₩12,345");
        assert_eq!(format(0.0, "KRW"), "₩0");
        assert_eq!(format(1234.5, "USD"), "$1,234.50");
        assert_eq!(format(3.0, "CHF"), "3.00 CHF");
        assert_eq!(format(-100.0, "KRW"), "-₩100");
        assert_eq!(format(-1234.5, "USD"), "-$1,234.50");
        assert_eq!(format(-0.001, "USD"), "$0.00");
    }

    #[test]
    fn parses_budget_notification() {
        let data = STANDARD.encode(r#"{"budgetDisplayName":"x","costAmount":1520.5,"costIntervalStart":"2026-11-01T07:00:00Z","budgetAmount":30000.0,"currencyCode":"KRW"}"#);
        let c = parse(&data, "2026-11-01T12:00:00.123Z").unwrap();
        assert_eq!(c.amount, 1520.5);
        assert_eq!(c.currency, "KRW");
        assert_eq!(c.interval_start, 1793516400.0);
        assert_eq!(c.published, 1793534400.123);
    }

    #[test]
    fn old_period_reads_zero() {
        let c = ApiCost { amount: 500.0, currency: "KRW".into(), interval_start: now() - 40.0 * 86400.0, published: 0.0 };
        assert_eq!(c.current(), 0.0);
        let c = ApiCost { interval_start: now() - 86400.0, ..c };
        assert_eq!(c.current(), 500.0);
    }
}
