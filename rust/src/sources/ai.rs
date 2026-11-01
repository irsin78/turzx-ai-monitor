//! AI plan usage (Claude, Codex, Antigravity): read-only, never writes credentials.
//! Requests follow CodexBar (MIT); Antigravity's token comes from Windows Credential
//! Manager as in Claude-Code-Usage-Monitor (MIT). All endpoints are
//! unofficial and may change.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use regex::bytes::Regex as BytesRegex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const UA: &str = "turzx-dashboard";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    pub session: Option<f64>,       // 5-hour window, percent used
    pub session_reset: Option<f64>, // epoch seconds
    pub weekly: Option<f64>,
    pub weekly_reset: Option<f64>,
    #[serde(default)]
    pub source: String,
    /// Non-secret plan fields as the service reports them (`plan_label` turns them into a name)
    #[serde(default)]
    pub plan: Value,
    /// Codex: rate-limit reset credits ("refill coupons") left
    #[serde(default)]
    pub coupons: u64,
    /// A second weekly limit inside the plan, percent used (Antigravity: Gemini 4 Argon)
    #[serde(default)]
    pub weekly_extra: Option<f64>,
}

/// Claude's plan from its login file: only the two plan fields are kept, never the tokens.
fn claude_plan() -> Value {
    let creds: Value = std::fs::read(home().join(r".claude\.credentials.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let o = &creds["claudeAiOauth"];
    json!({"rateLimitTier": plan_field(o, "rateLimitTier"), "subscriptionType": plan_field(o, "subscriptionType")})
}

fn plan_field<'a>(j: &'a Value, key: &str) -> Option<&'a str> {
    j[key].as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// The plan name shown next to the service name ("Max 5x", "Pro", "Free"): `[plan_labels]` first,
/// then a readable form of what the service reports.
pub fn plan_label(name: &str, plan: &Value) -> Option<String> {
    plan_label_with(name, plan, &crate::config::get().plan_labels)
}

fn plan_label_with(name: &str, plan: &Value, labels: &crate::config::PlanLabels) -> Option<String> {
    use crate::config::{readable_plan, PlanLabels};
    match name {
        "Claude" => {
            let tier = plan_field(plan, "rateLimitTier");
            let subscription = plan_field(plan, "subscriptionType");
            for raw in [tier, subscription].into_iter().flatten() {
                if let Some(l) = PlanLabels::lookup(&labels.claude, raw) {
                    return Some(l.into());
                }
            }
            match tier {
                Some(t) if t.contains("max_20x") => Some("Max 20x".into()),
                Some(t) if t.contains("max_5x") => Some("Max 5x".into()),
                _ => subscription.or(tier).map(readable_plan),
            }
        }
        "Codex" => plan_field(plan, "plan_type").map(|raw| PlanLabels::lookup(&labels.codex, raw).map_or_else(|| readable_plan(raw), str::to_owned)),
        "Antigravity" => {
            let mut fallback = None;
            // the paid tier first: a Google AI Pro account reports currentTier "free-tier" for Antigravity
            for key in ["paidTier", "currentTier"] {
                let Some(id) = plan_field(plan, key) else { continue };
                if let Some(l) = PlanLabels::lookup(&labels.antigravity, id) {
                    return Some(l.into());
                }
                if id == "free-tier" {
                    return Some("Free".into());
                }
                let tier_name = plan_field(plan, &format!("{key}Name")).map(|n| ["Antigravity ", "Google AI ", "Gemini "].iter().fold(n, |n, p| n.strip_prefix(p).unwrap_or(n)).trim());
                if let Some(n) = tier_name.filter(|n| !n.is_empty() && *n != "Antigravity") {
                    return Some(n.into());
                }
                fallback.get_or_insert_with(|| readable_plan(id));
            }
            fallback
        }
        _ => None,
    }
}

/// The usage API asked us to wait (HTTP 429 + Retry-After).
#[derive(Debug)]
pub struct RateLimited(pub f64);
impl std::fmt::Display for RateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "rate limited until {:.0}", self.0)
    }
}
impl std::error::Error for RateLimited {}

/// Non-success HTTP status.
#[derive(Debug)]
pub struct HttpStatus(pub u16);
impl std::fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "HTTP {}", self.0)
    }
}
impl std::error::Error for HttpStatus {}

/// The CLI is not signed in to a plan (login screen, or an API-billing login whose /usage has
/// no plan limits): the user has to sign in again.
#[derive(Debug)]
pub struct SignedOut;
impl std::fmt::Display for SignedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "not signed in to a plan")
    }
}
impl std::error::Error for SignedOut {}

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64()
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
}

pub(crate) fn local_appdata() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default())
}

pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into()
}

struct Reply {
    status: u16,
    body: Value,
}

fn send(req: ureq::RequestBuilder<ureq::typestate::WithoutBody>) -> Result<Reply> {
    let mut r = req.call()?;
    let status = r.status().as_u16();
    let body = r.body_mut().read_json().unwrap_or(Value::Null);
    Ok(Reply { status, body })
}

fn send_json(req: ureq::RequestBuilder<ureq::typestate::WithBody>, body: &Value) -> Result<Reply> {
    let mut r = req.send_json(body)?;
    let status = r.status().as_u16();
    let body = r.body_mut().read_json().unwrap_or(Value::Null);
    Ok(Reply { status, body })
}

fn iso(v: &Value) -> Option<f64> {
    chrono::DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|t| t.timestamp_millis() as f64 / 1000.0)
}

// --- cache ----------------------------------------------------------------------

/// Last results and rate-limit waits on disk, so a restart doesn't re-query.
/// Same file and format as the Python version (usage numbers and timestamps, no tokens).
#[derive(Clone)]
pub struct Cache {
    path: PathBuf,
    data: Arc<Mutex<HashMap<String, Value>>>,
}

impl Cache {
    pub fn open() -> Self {
        let path = local_appdata().join(r"TurzxDashboard\ai_cache.json");
        let data = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Cache { path, data: Arc::new(Mutex::new(data)) }
    }

    fn save(&self, data: &HashMap<String, Value>) {
        let _ = std::fs::create_dir_all(self.path.parent().unwrap());
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec(data).unwrap()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    pub fn last(&self, name: &str) -> (Option<Usage>, f64) {
        let d = self.data.lock().unwrap();
        let e = d.get(name);
        let u = e.and_then(|e| serde_json::from_value(e["usage"].clone()).ok());
        (u, e.and_then(|e| e["fetched_at"].as_f64()).unwrap_or(0.0))
    }

    pub fn store(&self, name: &str, u: &Usage) {
        let mut d = self.data.lock().unwrap();
        let e = d.entry(name.to_string()).or_insert_with(|| json!({}));
        e["usage"] = serde_json::to_value(u).unwrap();
        e["fetched_at"] = json!(now());
        let snapshot = d.clone();
        drop(d);
        self.save(&snapshot);
    }

}

// --- Claude ---------------------------------------------------------------------

/// How to start Claude Code: its native binary (official installer in ~/.local/bin, winget,
/// or a claude.exe on PATH), else the npm shim claude.cmd through cmd.exe. The npm package's
/// own claude.exe is a stub that pops up a "16-bit app" dialog, so npm directories are only
/// used for claude.cmd.
fn claude_argv() -> Vec<std::ffi::OsString> {
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let is_npm = |p: &PathBuf| p.to_string_lossy().to_lowercase().contains("npm");
    let native = [home().join(r".local\bin\claude.exe"), local_appdata().join(r"Microsoft\WinGet\Links\claude.exe")]
        .into_iter()
        .chain(path_dirs.iter().map(|d| d.join("claude.exe")).filter(|p| !is_npm(p)))
        .find(|p| p.is_file());
    if let Some(exe) = native {
        return vec![exe.into()];
    }
    let npm_dir = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("npm"));
    match npm_dir.into_iter().chain(path_dirs).map(|d| d.join("claude.cmd")).find(|p| p.is_file()) {
        Some(cmd) => vec!["cmd.exe".into(), "/c".into(), cmd.into()],
        None => vec![home().join(r".local\bin\claude.exe").into()],
    }
}

/// Parse Claude Code's /usage panel:
///   Current session ... 22% used / Resets 2:39am (Asia/Seoul)
///   Current week (all models) ... 20% used / Resets Oct 7, 12:59am (Asia/Seoul)
pub fn parse_usage_screen(text: &str, now_utc: chrono::DateTime<chrono::Utc>) -> Usage {
    let section = |title: &str| -> (Option<f64>, Option<f64>) {
        let re = Regex::new(&format!(r"(?s){}.*?(\d+(?:\.\d+)?)% used.*?Resets ([^\n(]+?)\s*\(([^)]+)\)", regex::escape(title))).unwrap();
        match re.captures(text) {
            Some(c) => (c[1].parse().ok(), reset_time(c[2].trim(), &c[3], now_utc)),
            None => (None, None),
        }
    };
    let (session, session_reset) = section("Current session");
    let (weekly, weekly_reset) = section("Current week (all models)");
    Usage { session, session_reset, weekly, weekly_reset, source: "cli".into(), plan: Value::Null, ..Default::default() }
}

/// "2:39am" (next occurrence) or "Oct 7, 12:59am" (this year, else next) -> epoch seconds.
fn reset_time(when: &str, tz: &str, now_utc: chrono::DateTime<chrono::Utc>) -> Option<f64> {
    use chrono::{Datelike, Duration as D, NaiveDate, NaiveTime, TimeZone};
    let zone: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::Asia::Seoul);
    let now = now_utc.with_timezone(&zone);
    let s = Regex::new(r"(?i)\s*(am|pm)$").unwrap().replace(when.trim(), "$1").to_uppercase();
    let (date_part, time_part) = match s.split_once(", ") {
        Some((d, t)) => (Some(d.to_string()), t.to_string()),
        None => (None, s.clone()),
    };
    let time = NaiveTime::parse_from_str(&time_part, "%I:%M%p")
        .or_else(|_| NaiveTime::parse_from_str(&format!("{}", time_part.replace("AM", ":00AM").replace("PM", ":00PM")), "%I:%M%p"))
        .ok()?;
    let local = |date: NaiveDate| zone.from_local_datetime(&date.and_time(time)).earliest();
    let dt = match date_part {
        Some(d) => {
            let md = NaiveDate::parse_from_str(&format!("{} {}", d, now.year()), "%b %d %Y").ok()?;
            let mut dt = local(md)?;
            if dt < now - D::days(1) {
                dt = local(md.with_year(now.year() + 1)?)?;
            }
            dt
        }
        None => {
            let mut dt = local(now.date_naive())?;
            if dt <= now {
                dt = local(now.date_naive() + D::days(1))?;
            }
            dt
        }
    };
    Some(dt.timestamp() as f64)
}

/// Fallback when the usage API fails (CodexBar's "CLI" source): run Claude Code in a hidden
/// pseudo-console, open /usage, read the panel, quit. /usage does not call a model. Runs in
/// its own empty folder, whose one-time "trust this folder" prompt is answered Yes.
pub struct ClaudeCli;

impl ClaudeCli {
    const COLS: u16 = 120;
    const ROWS: u16 = 50;

    pub fn fetch(&self, timeout: Duration) -> Result<Usage> {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};
        let work = local_appdata().join(r"TurzxDashboard\claude-probe");
        std::fs::create_dir_all(&work)?;
        let pair = native_pty_system().openpty(PtySize { rows: Self::ROWS, cols: Self::COLS, pixel_width: 0, pixel_height: 0 })?;
        let mut cmd = CommandBuilder::from_argv(claude_argv());
        cmd.cwd(&work);
        // Clean environment: never inherit another Claude Code session's variables
        cmd.env_clear();
        for (k, v) in std::env::vars_os() {
            let up = k.to_string_lossy().to_uppercase();
            if !up.starts_with("CLAUDE") && !up.starts_with("ANTHROPIC") {
                cmd.env(k, v);
            }
        }
        let mut child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);
        let parser = Arc::new(Mutex::new(vt100::Parser::new(Self::ROWS, Self::COLS, 0)));
        let mut reader = pair.master.try_clone_reader()?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer()?));
        let (p2, w2) = (parser.clone(), writer.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 65536];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let chunk = &buf[..n];
                // ConPTY asks for the cursor position at start-up and waits for the answer
                if chunk.windows(4).any(|w| w == b"\x1b[6n") {
                    let _ = w2.lock().unwrap().write_all(b"\x1b[1;1R");
                }
                p2.lock().unwrap().process(chunk);
            }
        });
        let send = |bytes: &[u8]| -> Result<()> { Ok(writer.lock().unwrap().write_all(bytes)?) };
        let text = || parser.lock().unwrap().screen().contents();
        let result = (|| -> Result<Usage> {
            let wait_for = |pred: &dyn Fn(&str) -> bool, secs: u64| -> bool {
                let end = Instant::now() + Duration::from_secs(secs);
                while Instant::now() < end {
                    if pred(&text()) {
                        return true;
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
                false
            };
            // Input prompt: "❯" (fullscreen renderer) or "> " (inline renderer, admin consoles).
            // A full-width rule above it is not followed by a newline in the screen text, so the
            // prompt may come right after "─".
            let prompt = Regex::new(r"(?m)(?:^|─)\s*(?:❯|>)\s").unwrap();
            let trust = |s: &str| s.to_lowercase().contains("trust this folder");
            // login screen, "Not logged in · Please run /login", or a header of
            // "Opus · API Usage Billing" (the plan login was lost)
            let signed_out = |s: &str| {
                let s = s.to_lowercase();
                ["api usage billing", "login method", "run /login", "not logged in"].iter().any(|k| s.contains(k))
            };
            if !wait_for(&|s| trust(s) || prompt.is_match(s) || signed_out(s), 20) {
                bail!("Claude Code did not start");
            }
            if signed_out(&text()) {
                return Err(SignedOut.into());
            }
            if trust(&text()) {
                send(b"\x1b[B")?; // "Yes, I trust this folder"
                std::thread::sleep(Duration::from_millis(400));
                send(b"\r")?;
                if !wait_for(&|s| !trust(s), 15) {
                    bail!("trust prompt did not close");
                }
                std::thread::sleep(Duration::from_millis(1500));
            }
            send(b"/usage")?;
            std::thread::sleep(Duration::from_millis(800));
            send(b"\r")?;
            let done = |s: &str| s.contains("Current week") && s.matches("% used").count() >= 2 && s.matches("Resets").count() >= 2;
            if !wait_for(&|s| done(s) || signed_out(s), timeout.as_secs()) || !done(&text()) {
                if signed_out(&text()) {
                    return Err(SignedOut.into());
                }
                bail!("no plan usage on /usage screen");
            }
            std::thread::sleep(Duration::from_millis(500)); // let the last line render
            let mut u = parse_usage_screen(&text(), chrono::Utc::now());
            if u.session.is_none() && u.weekly.is_none() {
                bail!("could not parse /usage screen");
            }
            u.plan = claude_plan();
            Ok(u)
        })();
        if result.is_err() {
            let path = local_appdata().join(r"TurzxDashboard\claude_cli_debug.txt");
            let _ = std::fs::write(path, text());
        }
        let _ = send(b"\x1b");
        let _ = child.kill();
        result
    }
}

// --- Codex ----------------------------------------------------------------------

pub fn codex() -> Result<Usage> {
    let auth: Value = serde_json::from_slice(&std::fs::read(home().join(r".codex\auth.json"))?)?;
    let t = &auth["tokens"];
    let mut req = agent()
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("Authorization", format!("Bearer {}", t["access_token"].as_str().context("no Codex token")?))
        .header("User-Agent", UA)
        .header("Accept", "application/json");
    if let Some(acc) = t["account_id"].as_str() {
        req = req.header("ChatGPT-Account-Id", acc);
    }
    let r = send(req)?;
    if r.status != 200 {
        return Err(HttpStatus(r.status).into());
    }
    let rl = &r.body["rate_limit"];
    let (p, s) = (&rl["primary_window"], &rl["secondary_window"]);
    Ok(Usage {
        session: p["used_percent"].as_f64(),
        session_reset: p["reset_at"].as_f64(),
        weekly: s["used_percent"].as_f64(),
        weekly_reset: s["reset_at"].as_f64(),
        source: "api".into(),
        plan: json!({"plan_type": plan_field(&r.body, "plan_type")}),
        coupons: r.body["rate_limit_reset_credits"]["available_count"].as_u64().unwrap_or(0),
        weekly_extra: None,
    })
}

// --- Antigravity ----------------------------------------------------------------

/// Token from Windows Credential Manager ("gemini:antigravity"); an expired access token is
/// refreshed in memory only (agy's stored credential is left untouched).
pub struct Antigravity {
    token: Option<(String, f64)>,
    project: Option<String>,
    /// tier ids and names from loadCodeAssist
    plan: Value,
}

impl Antigravity {
    const BASE: &'static str = "https://daily-cloudcode-pa.googleapis.com/v1internal:";

    pub fn new() -> Self {
        Antigravity { token: None, project: None, plan: Value::Null }
    }

    fn stored() -> Result<Value> {
        use windows::core::PCWSTR;
        use windows::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};
        let target: Vec<u16> = "gemini:antigravity".encode_utf16().chain([0]).collect();
        let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        unsafe {
            CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut p).context("no Antigravity credential")?;
            let c = &*p;
            let blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec();
            CredFree(p as *const _);
            let v: Value = serde_json::from_slice(&blob)?;
            Ok(v["token"].clone())
        }
    }

    /// (client_id, client_secret) pairs embedded in the installed Antigravity binaries;
    /// secrets are exactly 35 chars (as Claude-Code-Usage-Monitor extracts them).
    fn oauth_clients() -> Vec<(String, String)> {
        static FOUND: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
        FOUND.get_or_init(Self::scan_clients).clone()
    }

    /// Scan the binaries in 64 KiB windows (they are ~200 MB; reading one whole made the
    /// working set jump by that much).
    fn scan_clients() -> Vec<(String, String)> {
        use std::io::Read;
        let la = local_appdata();
        let pf = PathBuf::from(std::env::var("ProgramFiles").unwrap_or_default());
        let id_re = BytesRegex::new(r"[0-9]{10,}-[0-9A-Za-z_-]{20,80}\.apps\.googleusercontent\.com").unwrap();
        let sec_re = BytesRegex::new(r"GOCSPX-[0-9A-Za-z_-]{28}").unwrap();
        for path in [
            la.join(r"Programs\Antigravity\resources\bin\language_server.exe"),
            la.join(r"agy\bin\agy.exe"),
            pf.join(r"Antigravity\resources\bin\language_server.exe"),
        ] {
            let Ok(mut file) = std::fs::File::open(path) else { continue };
            const WINDOW: usize = 64 * 1024;
            const OVERLAP: usize = 256; // longer than any match, so none is cut in two
            let mut buf = vec![0u8; WINDOW + OVERLAP];
            let (mut ids, mut secrets): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
            let mut kept = 0; // bytes carried over from the previous window
            loop {
                let n = match file.read(&mut buf[kept..]) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let data = &buf[..kept + n];
                for (re, out) in [(&id_re, &mut ids), (&sec_re, &mut secrets)] {
                    for m in re.find_iter(data) {
                        let s = String::from_utf8_lossy(m.as_bytes()).to_string();
                        if !out.contains(&s) {
                            out.push(s);
                        }
                    }
                }
                kept = OVERLAP.min(data.len());
                let len = data.len();
                buf.copy_within(len - kept..len, 0);
            }
            let pairs: Vec<_> = ids.iter().flat_map(|i| secrets.iter().map(move |s| (i.clone(), s.clone()))).collect();
            if !pairs.is_empty() {
                return pairs;
            }
        }
        Vec::new()
    }

    fn refresh(refresh_token: &str) -> Result<(String, f64)> {
        let mut errors = Vec::new();
        for (cid, sec) in Self::oauth_clients() {
            let mut r = agent().post("https://oauth2.googleapis.com/token").send_form([
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", cid.as_str()),
                ("client_secret", sec.as_str()),
            ])?;
            let status = r.status().as_u16();
            let j: Value = r.body_mut().read_json().unwrap_or(Value::Null);
            if status == 200 {
                let tok = j["access_token"].as_str().context("no access_token")?.to_string();
                return Ok((tok, now() + j["expires_in"].as_f64().unwrap_or(3600.0)));
            }
            errors.push(format!("{status} {}", j["error"].as_str().unwrap_or("")));
            if ![400, 401, 403].contains(&status) {
                break; // network/server trouble: retry next poll
            }
        }
        Err(anyhow!("Antigravity token refresh failed: {errors:?}"))
    }

    fn auth(&mut self) -> Result<String> {
        if let Some((t, exp)) = &self.token {
            if now() < exp - 60.0 {
                return Ok(t.clone());
            }
        }
        let s = Self::stored()?;
        let exp = iso(&s["expiry"]).unwrap_or(0.0);
        let tok = if now() < exp - 60.0 {
            (s["access_token"].as_str().context("no access_token")?.to_string(), exp)
        } else {
            Self::refresh(s["refresh_token"].as_str().context("no refresh_token")?)?
        };
        self.token = Some(tok.clone());
        Ok(tok.0)
    }

    fn post(&self, token: &str, method: &str, body: Value) -> Result<Value> {
        let r = send_json(
            agent()
                .post(format!("{}{method}", Self::BASE))
                .header("Authorization", format!("Bearer {token}"))
                .header("User-Agent", "antigravity"),
            &body,
        )?;
        if r.status != 200 {
            return Err(HttpStatus(r.status).into());
        }
        Ok(r.body)
    }

    fn quota(&mut self, token: &str) -> Result<Value> {
        self.post(token, "retrieveUserQuotaSummary", json!({"project": self.project}))
    }

    /// The quota summary and per-model quota as the server sends them, and the CLI's groups
    /// (no credentials in them), for `diag agy-raw`.
    pub fn quota_raw(&mut self) -> Result<Value> {
        self.load_plan()?; // signs in and finds the project
        let token = self.auth()?;
        let summary = self.quota(&token)?;
        let per_model = self.post(&token, "retrieveUserQuota", json!({"project": self.project})).unwrap_or_default();
        let cli = Self::cli_usage().unwrap_or_else(|e| json!(e.to_string()));
        Ok(json!({"summary": summary, "per_model": per_model["buckets"], "cli": cli}))
    }

    /// `agy -p /usage --output-format json`: the groups as the CLI shows them (the API summary
    /// leaves out the Gemini group). Calls no model; about 6 s. Runs in its own empty folder
    /// with its own log file, which agy overwrites each run.
    fn cli_usage() -> Result<Value> {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let path_dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        let exe = std::iter::once(local_appdata().join(r"agy\bin\agy.exe"))
            .chain(path_dirs.iter().map(|d| d.join("agy.exe")))
            .find(|p| p.is_file())
            .context("agy CLI not found")?;
        let work = local_appdata().join(r"TurzxDashboard\agy-probe");
        std::fs::create_dir_all(&work)?;
        let mut child = Command::new(exe)
            .args(["-p", "/usage", "--output-format", "json", "--print-timeout", "40s", "--log-file"])
            .arg(work.join("agy.log"))
            .current_dir(&work)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        let mut out = child.stdout.take().context("no stdout")?;
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        });
        let start = Instant::now();
        while child.try_wait()?.is_none() {
            if start.elapsed() > Duration::from_secs(50) {
                let _ = child.kill();
                bail!("agy /usage timed out");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let j: Value = serde_json::from_slice(&reader.join().unwrap_or_default()).context("agy /usage: not JSON")?;
        let groups = &j["command"]["data"]["groups"];
        if !groups.is_array() {
            bail!("agy /usage: {}", j["status"].as_str().unwrap_or("no quota groups"));
        }
        Ok(groups.clone())
    }

    /// The Gemini group's 5-hour and weekly windows (the Antigravity plan's own quota; the
    /// Claude/GPT group is what those models use inside Antigravity). Falls back to the first
    /// group that has either window.
    fn from_cli(groups: &Value, u: &mut Usage) -> bool {
        let groups = groups.as_array().map(Vec::as_slice).unwrap_or_default();
        let has = |g: &&Value, id: &str| g["buckets"].as_array().into_iter().flatten().any(|b| b["id"] == id);
        let Some(g) = groups
            .iter()
            .find(|g| has(g, "gemini-5h") || has(g, "gemini-weekly"))
            .or_else(|| groups.iter().find(|g| g["buckets"].as_array().into_iter().flatten().any(|b| b["window"] == "5h")))
        else {
            return false;
        };
        let used = |b: &Value| (1.0 - b["remaining_fraction"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0)) * 100.0;
        for b in g["buckets"].as_array().into_iter().flatten() {
            match b["window"].as_str() {
                Some("5h") => (u.session, u.session_reset) = (Some(used(b)), iso(&b["reset_time"])),
                Some("weekly") => (u.weekly, u.weekly_reset) = (Some(used(b)), iso(&b["reset_time"])),
                _ => {}
            }
        }
        // Gemini 4 Argon has its own weekly limit on paid Pro plans (its own group), likely for a
        // few months: once agy stops listing it this is None and the weekly cell is as before
        u.weekly_extra = groups.iter().flat_map(|g| g["buckets"].as_array().into_iter().flatten())
            .find(|b| b["window"] == "weekly" && b["id"].as_str().is_some_and(|id| id.contains("argon")))
            .map(used);
        u.session.is_some() || u.weekly.is_some()
    }

    /// The CLI's /usage when agy is installed, else the API. The plan name always comes from
    /// the API (loadCodeAssist).
    pub fn fetch(&mut self) -> Result<Usage> {
        let plan = self.load_plan();
        if let Ok(groups) = Self::cli_usage() {
            let mut u = Usage { source: "cli".into(), plan: self.plan.clone(), ..Default::default() };
            if Self::from_cli(&groups, &mut u) {
                return Ok(u);
            }
        }
        plan?;
        self.fetch_api()
    }

    fn load_plan(&mut self) -> Result<()> {
        let token = self.auth()?;
        if self.project.is_none() {
            let j = self.post(&token, "loadCodeAssist", json!({"metadata": {"ideType": "ANTIGRAVITY"}}))?;
            self.project = Some(j["cloudaicompanionProject"].as_str().context("no project")?.to_string());
            self.plan = json!({
                "currentTier": plan_field(&j["currentTier"], "id"),
                "currentTierName": plan_field(&j["currentTier"], "name"),
                "paidTier": plan_field(&j["paidTier"], "id"),
                "paidTierName": plan_field(&j["paidTier"], "name"),
            });
        }
        Ok(())
    }

    fn fetch_api(&mut self) -> Result<Usage> {
        let token = self.auth()?;
        if self.project.is_none() {
            let j = self.post(&token, "loadCodeAssist", json!({"metadata": {"ideType": "ANTIGRAVITY"}}))?;
            self.project = Some(j["cloudaicompanionProject"].as_str().context("no project")?.to_string());
            self.plan = json!({
                "currentTier": plan_field(&j["currentTier"], "id"),
                "currentTierName": plan_field(&j["currentTier"], "name"),
                "paidTier": plan_field(&j["paidTier"], "id"),
                "paidTierName": plan_field(&j["paidTier"], "name"),
            });
        }
        let j = self.quota(&token)?;
        // Two groups (Gemini, Claude/GPT) with separate limits: show the most used one per window
        let mut u = Usage { source: "api".into(), plan: self.plan.clone(), ..Default::default() };
        for g in j["groups"].as_array().into_iter().flatten() {
            for b in g["buckets"].as_array().into_iter().flatten() {
                let pct = (1.0 - b["remainingFraction"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0)) * 100.0;
                let (val, reset) = match b["window"].as_str().map(str::to_lowercase).as_deref() {
                    Some("5h") => (&mut u.session, &mut u.session_reset),
                    Some("weekly") => (&mut u.weekly, &mut u.weekly_reset),
                    _ => continue,
                };
                if val.is_none_or(|v| pct > v) {
                    *val = Some(pct);
                    *reset = iso(&b["resetTime"]);
                }
            }
        }
        // The summary leaves out the Gemini group; each model's own quota still has it. Two
        // calls give it, and replicas have been seen answering with an older value, so the most
        // used wins. A reset within 6 hours is the 5-hour window, a later one the weekly.
        let models = self.post(&token, "fetchAvailableModels", json!({"project": self.project})).unwrap_or_default();
        let buckets = self.post(&token, "retrieveUserQuota", json!({"project": self.project})).unwrap_or_default();
        let quotas = models["models"].as_object().into_iter().flat_map(|o| o.values()).map(|m| &m["quotaInfo"]);
        for q in quotas.chain(buckets["buckets"].as_array().into_iter().flatten()) {
            let (Some(left), Some(reset)) = (q["remainingFraction"].as_f64(), iso(&q["resetTime"])) else { continue };
            let pct = (1.0 - left.clamp(0.0, 1.0)) * 100.0;
            let (val, at) = if reset - now() < 6.0 * 3600.0 { (&mut u.session, &mut u.session_reset) } else { (&mut u.weekly, &mut u.weekly_reset) };
            if val.is_none_or(|v| pct > v) {
                *val = Some(pct);
                *at = Some(reset);
            }
        }
        Ok(u)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn plan_labels() {
        let none = crate::config::PlanLabels::default();
        let label = |name, plan| plan_label_with(name, &plan, &none);
        assert_eq!(label("Claude", json!({"rateLimitTier": "default_claude_ai", "subscriptionType": "pro"})).as_deref(), Some("Pro"));
        assert_eq!(label("Claude", json!({"rateLimitTier": "default_claude_max_5x", "subscriptionType": "max"})).as_deref(), Some("Max 5x"));
        assert_eq!(label("Codex", json!({"plan_type": "plus"})).as_deref(), Some("Plus"));
        let agy = json!({"currentTier": "free-tier", "currentTierName": "Antigravity", "paidTier": "g1-pro-tier", "paidTierName": "Google AI Pro"});
        assert_eq!(label("Antigravity", agy).as_deref(), Some("Pro"));
        assert_eq!(label("Antigravity", json!({"currentTier": "free-tier"})).as_deref(), Some("Free"));
        assert_eq!(label("Codex", Value::Null), None);
        let mut custom = crate::config::PlanLabels::default();
        custom.codex.insert("pro".into(), "Pro 200".into());
        assert_eq!(plan_label_with("Codex", &json!({"plan_type": "pro"}), &custom).as_deref(), Some("Pro 200"));
    }

    #[test]
    fn antigravity_cli_uses_gemini_group() {
        let groups = json!([
            {"name": "Claude and GPT models", "buckets": [
                {"id": "3p-weekly", "window": "weekly", "remaining_fraction": 1, "reset_time": "2026-11-07T12:49:22Z"},
                {"id": "3p-5h", "window": "5h", "remaining_fraction": 1, "reset_time": "2026-10-31T17:49:22Z"}]},
            {"name": "Gemini Models", "buckets": [
                {"id": "gemini-weekly", "window": "weekly", "remaining_fraction": 0.99, "reset_time": "2026-11-07T12:03:38Z"},
                {"id": "gemini-5h", "window": "5h", "remaining_fraction": 0.75, "reset_time": "2026-10-31T17:03:38Z"}]},
            {"name": "Gemini 4 Argon", "buckets": [
                {"id": "gemini-argon-wtu-weekly", "window": "weekly", "remaining_fraction": 0.5, "reset_time": "2026-11-07T11:41:25Z"}]}
        ]);
        let mut u = Usage::default();
        assert!(Antigravity::from_cli(&groups, &mut u));
        assert!((u.session.unwrap() - 25.0).abs() < 1e-6 && (u.weekly.unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(u.weekly_reset, iso(&json!("2026-11-07T12:03:38Z")));
        assert!((u.weekly_extra.unwrap() - 50.0).abs() < 1e-6);
        // without the Argon group: no second weekly value, nothing else changes
        let gone = json!([groups[0].clone(), groups[1].clone()]);
        let mut v = Usage::default();
        assert!(Antigravity::from_cli(&gone, &mut v));
        assert_eq!((v.weekly_extra, v.session, v.weekly), (None, u.session, u.weekly));
        assert!(!Antigravity::from_cli(&json!("error"), &mut Usage::default()));
    }

    #[test]
    fn parses_usage_panel_and_year_rollover() {
        let sample = "Current session\n ███   22% used\n Resets 2:39am (Asia/Seoul)\n\n\
                      Current week (all models)\n ██   20% used\n Resets Oct 7, 12:59am (Asia/Seoul)\n";
        let seoul = chrono_tz::Asia::Seoul;
        let now = seoul.with_ymd_and_hms(2026, 10, 1, 23, 5, 0).unwrap().with_timezone(&chrono::Utc);
        let u = parse_usage_screen(sample, now);
        let fmt = |t: f64| chrono::DateTime::from_timestamp(t as i64, 0).unwrap().with_timezone(&seoul).format("%Y-%m-%d %H:%M").to_string();
        assert_eq!((u.session, u.weekly), (Some(22.0), Some(20.0)));
        assert_eq!(fmt(u.session_reset.unwrap()), "2026-10-02 02:39");
        assert_eq!(fmt(u.weekly_reset.unwrap()), "2026-10-07 00:59");
        let dec = seoul.with_ymd_and_hms(2026, 12, 30, 23, 0, 0).unwrap().with_timezone(&chrono::Utc);
        let u = parse_usage_screen(&sample.replace("Oct 7", "Jan 2"), dec);
        assert_eq!(fmt(u.weekly_reset.unwrap()), "2027-01-02 00:59");
    }
}
