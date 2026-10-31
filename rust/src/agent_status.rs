//! Claude Code / Codex sessions that finished or wait for the user, for a bubble over their
//! mascots. Claude Code hooks and Codex's notify run `agent-event`, which sends one small
//! loopback UDP datagram and exits; the dashboard keeps the sessions in memory. No transcript or
//! assistant message is forwarded. (After the 3.5" turzxforai35 dashboard.)
use std::collections::HashMap;
use std::io::Read;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

/// 47136, not the 3.5" dashboard's 47135: both may run on one PC
pub const DEFAULT_PORT: u16 = 47136;
pub const MARKER: &str = "--turzx-agent-status-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source { Claude, Codex }
impl Source { pub fn index(self) -> usize { match self { Self::Claude => 0, Self::Codex => 1 } } }

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status { Clear, Done, Waiting }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub source: Source,
    pub event: Status,
    pub session: String,
    pub project: String,
}
impl Event {
    fn valid(&self) -> bool {
        !self.session.is_empty() && self.session.len() <= 256 && self.project.chars().count() <= 100
            && !self.session.chars().chain(self.project.chars()).any(char::is_control)
            && !(self.source == Source::Codex && self.event == Status::Waiting)
    }
    pub fn encode(&self) -> Option<Vec<u8>> { self.valid().then(|| serde_json::to_vec(self).ok()).flatten() }
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() > 4096 { return None; }
        let value: Self = serde_json::from_slice(data).ok()?;
        value.valid().then_some(value)
    }
}

fn is_wrap_up(v: &serde_json::Value) -> bool {
    v["tool_name"].as_str().is_some_and(|t| t.ends_with("__wrap_up"))
}

/// A tool that stops for the user's answer.
fn asks_user(v: &serde_json::Value) -> bool {
    match v["tool_name"].as_str().unwrap_or("") {
        "AskUserQuestion" | "ExitPlanMode" => true,
        _ => is_wrap_up(v) && v["tool_input"]["ask"].as_str().is_some_and(|a| !a.trim().is_empty()),
    }
}

pub fn parse(source: Source, input: &str) -> Option<Event> {
    let v: serde_json::Value = serde_json::from_str(input).ok()?;
    let (session, status) = match source {
        Source::Claude => {
            let status = match v["hook_event_name"].as_str()? {
                "Notification" => {
                    let kind = v["notification_type"].as_str().unwrap_or("");
                    let message = v["message"].as_str().unwrap_or("").to_ascii_lowercase();
                    if !["permission_prompt", "idle_prompt", "elicitation_dialog"].contains(&kind)
                        && !message.contains("permission") && !message.contains("waiting for your input")
                        && !message.contains("needs your input") { return None; }
                    Status::Waiting
                }
                // The desktop app sends no Notification: its permission dialogs come as
                // PermissionRequest, its questions as tools (AskUserQuestion, plan approval, and a
                // turn-end question box: the ccd wrap_up tool with an `ask`)
                "PermissionRequest" => Status::Waiting,
                "PreToolUse" if asks_user(&v) => Status::Waiting,
                "Stop" => Status::Done,
                // wrap_up returns as soon as its box is up: that is not an answer
                "PostToolUse" if is_wrap_up(&v) => return None,
                // PostToolUse: a tool ran, so an answered prompt or question is no longer waiting
                "UserPromptSubmit" | "SessionEnd" | "PostToolUse" => Status::Clear,
                _ => return None,
            };
            (v["session_id"].as_str()?, status)
        }
        Source::Codex => {
            if v["type"] != "agent-turn-complete" { return None; }
            (v["thread-id"].as_str()?, Status::Done)
        }
    };
    // Split both separators, independent of the shell that delivered cwd.
    let project = v["cwd"].as_str().unwrap_or("").trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\']).next().unwrap_or("").chars().filter(|c| !c.is_control()).take(100).collect();
    let event = Event { source, event: status, session: session.into(), project };
    event.valid().then_some(event)
}

#[derive(Clone)]
struct Session { event: Event, at: Instant, sequence: u64 }
#[derive(Clone, Default)]
pub struct Sessions { entries: HashMap<(Source, String), Session>, pub revision: [u64; 2] }
pub struct Summary<'a> { pub status: Status, pub project: &'a str, pub more: usize }
impl Sessions {
    pub fn apply(&mut self, event: Event, now: Instant) {
        self.expire(now);
        let key = (event.source, event.session.clone());
        if event.event == Status::Clear { self.entries.remove(&key); return; }
        // a turn that ends on a question (or a pending prompt) is still waiting: Stop's done must
        // not hide it; only an answer (Clear) or expiry ends it
        if event.event == Status::Done && self.entries.get(&key).is_some_and(|s| s.event.event == Status::Waiting) { return; }
        let index = event.source.index();
        self.revision[index] = self.revision[index].wrapping_add(1);
        if self.entries.len() >= 256 && !self.entries.contains_key(&key)
            && let Some(old) = self.entries.iter().min_by_key(|(_, s)| s.at).map(|(k, _)| k.clone()) {
            self.entries.remove(&old);
        }
        self.entries.insert(key, Session { event, at: now, sequence: self.revision[index] });
    }
    /// Waiting stays until answered (or 30 min). Claude's done was seen as it arrived: 10 s. Codex
    /// sends no "next prompt" event, so its done has nothing to clear it but time: 3 min.
    fn lifetime(e: &Event) -> Duration {
        Duration::from_secs(match (e.event, e.source) {
            (Status::Waiting, _) => 1800,
            (_, Source::Claude) => 10,
            (_, Source::Codex) => 180,
        })
    }
    fn alive(s: &Session, now: Instant) -> bool {
        now.saturating_duration_since(s.at) < Self::lifetime(&s.event)
    }
    fn expire(&mut self, now: Instant) { self.entries.retain(|_, s| Self::alive(s, now)); }
    pub fn summary(&self, source: Source, now: Instant) -> Option<Summary<'_>> {
        let mut all: Vec<_> = self.entries.values().filter(|s| s.event.source == source && Self::alive(s, now)).collect();
        all.sort_by_key(|s| (s.event.event, s.sequence));
        let s = all.last()?;
        Some(Summary { status: s.event.event, project: &s.event.project, more: all.len() - 1 })
    }
}

pub fn listen(state: &Arc<Mutex<crate::state::State>>, port: u16) {
    let socket = match UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, port)) {
        Ok(s) => s,
        Err(e) => { log::warn!("agent-status UDP {port}: {e}"); return; }
    };
    let state = Arc::clone(state);
    std::thread::Builder::new().name("agent-status".into()).spawn(move || {
        // On Windows recv_from fails with WSAEMSGSIZE if the datagram exceeds the
        // buffer. Receive the whole maximum UDP payload, then enforce our 4 KiB limit.
        let mut buf = [0; 65535];
        loop {
            match receive(&socket, &mut buf) {
                Ok(Some(event)) => state.lock().unwrap().agents.apply(event, Instant::now()),
                Ok(None) => {},
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted || e.raw_os_error() == Some(10040) => continue,
                Err(e) => { log::warn!("agent-status receive: {e}"); break; }
            }
        }
    }).expect("spawn agent-status listener");
}

fn receive(socket: &UdpSocket, buf: &mut [u8]) -> std::io::Result<Option<Event>> {
    let (len, peer) = socket.recv_from(buf)?;
    Ok(if peer.ip().is_loopback() { Event::decode(&buf[..len]) } else { None })
}

/// This runs before console attachment, logging and settings loading. The Windows GUI
/// subsystem preserves redirected standard handles; AttachConsole could replace them.
/// The previous notify is spawned with no console, and never awaited.
pub fn command(args: &[String]) {
    let Some(source) = args.first().and_then(|s| match s.as_str() { "claude" => Some(Source::Claude), "codex" => Some(Source::Codex), _ => None }) else { return; };
    let option = |key| args.iter().position(|s| s == key).and_then(|i| args.get(i + 1));
    let port = option("--port").and_then(|s| s.parse().ok()).filter(|&p| p != 0).unwrap_or(DEFAULT_PORT);
    let input = match source {
        Source::Claude => {
            let mut input = String::new();
            if std::io::stdin().take(1024 * 1024).read_to_string(&mut input).is_err() { return; }
            input
        }
        Source::Codex => {
            let Some(input) = args.last() else { return; };
            if let Some(raw) = option("--previous-notify") { forward_notify(raw, input); }
            input.clone()
        }
    };
    let parsed = parse(source, &input);
    trace(source, &input, parsed.as_ref());
    if let Some(bytes) = parsed.and_then(|e| e.encode())
        && let Ok(socket) = UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0)) {
        let _ = socket.set_nonblocking(true);
        let _ = socket.send_to(&bytes, (std::net::Ipv4Addr::LOCALHOST, port));
    }
}

/// One line per hook call in %LOCALAPPDATA%\TurzxDashboard\agent-events.log (kept under 64 KB):
/// which events arrive and what they became. Event names, notification type and text, project.
fn trace(source: Source, input: &str, parsed: Option<&Event>) {
    use std::io::Write;
    let Some(base) = std::env::var_os("LOCALAPPDATA") else { return; };
    let path = std::path::PathBuf::from(base).join("TurzxDashboard").join("agent-events.log");
    let v: serde_json::Value = serde_json::from_str(input).unwrap_or_default();
    let field = |k: &str| v[k].as_str().unwrap_or("").chars().filter(|c| !c.is_control()).take(120).collect::<String>();
    let what = match source {
        Source::Claude => format!("claude {} type={:?} message={:?}", field("hook_event_name"), field("notification_type"), field("message")),
        Source::Codex => format!("codex {}", field("type")),
    };
    let result = parsed.map_or("ignored".to_string(), |e| format!("{:?} {}", e.event, e.project));
    let truncate = std::fs::metadata(&path).is_ok_and(|m| m.len() > 64 * 1024);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(!truncate).write(true).truncate(truncate).open(&path) {
        let _ = writeln!(f, "{} {what} -> {result}", chrono::Local::now().format("%m-%d %H:%M:%S"));
    }
}

fn forward_notify(raw: &str, input: &str) {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    if let Ok(v) = toml::from_str::<toml::Table>(&format!("notify = {raw}"))
        && let Some(argv) = v.get("notify").and_then(toml::Value::as_array)
        && let Some(exe) = argv.first().and_then(toml::Value::as_str) {
        let args: Option<Vec<_>> = argv[1..].iter().map(toml::Value::as_str).collect();
        if let Some(args) = args {
            // Windows hands every inheritable handle to a child: the helper would keep our standard
            // handles (pipes Codex may wait on) open for as long as it runs
            unsafe {
                use windows::Win32::Foundation::{SetHandleInformation, HANDLE_FLAGS, HANDLE_FLAG_INHERIT};
                use windows::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
                for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                    if let Ok(h) = GetStdHandle(which) && !h.is_invalid() {
                        let _ = SetHandleInformation(h, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0));
                    }
                }
            }
            let _ = Command::new(exe).args(args).arg(input).stdin(Stdio::null()).stdout(Stdio::null())
                .stderr(Stdio::null()).creation_flags(0x0800_0000).spawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hook_events_and_private_data() {
        for (hook, status) in [("Notification", Status::Waiting), ("Stop", Status::Done), ("UserPromptSubmit", Status::Clear), ("SessionEnd", Status::Clear), ("PostToolUse", Status::Clear)] {
            let input = serde_json::json!({"hook_event_name":hook,"session_id":"abc","cwd":"C:\\Projects\\turzx35\\","message":"Claude needs your permission to use Bash", "transcript_path":"secret"});
            let event = parse(Source::Claude, &input.to_string()).unwrap();
            assert_eq!(event.event, status); assert_eq!(event.project, "turzx35");
            assert!(!String::from_utf8(event.encode().unwrap()).unwrap().contains("secret"));
        }
        for message in ["Claude is waiting for your input", "Claude needs your input"] {
            assert!(parse(Source::Claude, &serde_json::json!({"hook_event_name":"Notification","message":message,"session_id":"s"}).to_string()).is_some());
        }
        assert!(parse(Source::Claude, r#"{"hook_event_name":"Notification","session_id":"a","message":"login successful"}"#).is_none());
        assert!(parse(Source::Claude, r#"{"hook_event_name":"Notification","session_id":"a","notification_type":"permission_prompt"}"#).is_some());
        let e = parse(Source::Codex, r#"{"type":"agent-turn-complete","thread-id":"t","cwd":"/work/proj","last-assistant-message":"private"}"#).unwrap();
        assert_eq!(e.project, "proj"); assert_eq!(Event::decode(&e.encode().unwrap()), Some(e));
        for bad in [b"junk".as_slice(), br#"{"source":"unknown"}"#, br#"{"source":"codex","event":"waiting","session":"s","project":"p"}"#] { assert!(Event::decode(bad).is_none()); }
        assert!(parse(Source::Codex, r#"{"type":"other","thread-id":"t"}"#).is_none());
        assert!(parse(Source::Claude, "{}").is_none());
    }
    #[test]
    fn priority_counts_expiry_and_clear() {
        let now = Instant::now(); let mut s = Sessions::default();
        let event = |id: &str, status| Event { source: Source::Claude, event: status, session: id.into(), project: id.into() };
        s.apply(event("wait", Status::Waiting), now);
        s.apply(event("done", Status::Done), now + Duration::from_secs(1));
        s.apply(event("other", Status::Done), now + Duration::from_secs(2));
        let summary = s.summary(Source::Claude, now).unwrap();
        assert_eq!((summary.project, summary.more), ("wait", 2));
        assert_eq!(s.summary(Source::Claude, now + Duration::from_secs(602)).unwrap().more, 0);
        assert!(s.summary(Source::Claude, now + Duration::from_secs(1800)).is_none());
        s.apply(event("wait", Status::Clear), now);
        assert_eq!(s.summary(Source::Claude, now).unwrap().project, "other");
        assert!(s.summary(Source::Codex, now).is_none());
    }
    #[test]
    fn questions_wait_until_answered() {
        let claude = |json: &str| parse(Source::Claude, json).map(|e| e.event);
        let hook = |name: &str, tool: &str, input: &str| format!(r#"{{"hook_event_name":"{name}","session_id":"s","cwd":"/p","tool_name":"{tool}","tool_input":{input}}}"#);
        assert_eq!(claude(&hook("PreToolUse", "AskUserQuestion", "{}")), Some(Status::Waiting));
        assert_eq!(claude(&hook("PreToolUse", "ExitPlanMode", "{}")), Some(Status::Waiting));
        assert_eq!(claude(&hook("PreToolUse", "mcp__ccd_turn__wrap_up", r#"{"recap":"r","ask":"Push?"}"#)), Some(Status::Waiting));
        assert_eq!(claude(&hook("PreToolUse", "mcp__ccd_turn__wrap_up", r#"{"recap":"r"}"#)), None);
        assert_eq!(claude(&hook("PreToolUse", "Bash", "{}")), None);
        assert_eq!(claude(&hook("PostToolUse", "mcp__ccd_turn__wrap_up", "{}")), None);
        assert_eq!(claude(&hook("PostToolUse", "AskUserQuestion", "{}")), Some(Status::Clear));
        assert_eq!(claude(r#"{"hook_event_name":"PermissionRequest","session_id":"s","tool_name":"Bash"}"#), Some(Status::Waiting));
        // Stop after a question keeps it waiting; the next prompt clears it
        let now = Instant::now(); let mut s = Sessions::default();
        let ev = |status| Event { source: Source::Claude, event: status, session: "s".into(), project: "p".into() };
        s.apply(ev(Status::Waiting), now);
        s.apply(ev(Status::Done), now);
        assert_eq!(s.summary(Source::Claude, now + Duration::from_secs(60)).unwrap().status, Status::Waiting);
        s.apply(ev(Status::Clear), now);
        assert!(s.summary(Source::Claude, now).is_none());
    }
    #[test]
    fn done_lifetimes() {
        let now = Instant::now(); let mut s = Sessions::default();
        let done = |source| Event { source, event: Status::Done, session: "s".into(), project: "p".into() };
        s.apply(done(Source::Claude), now);
        s.apply(done(Source::Codex), now);
        assert!(s.summary(Source::Claude, now + Duration::from_secs(9)).is_some());
        assert!(s.summary(Source::Claude, now + Duration::from_secs(10)).is_none());
        assert!(s.summary(Source::Codex, now + Duration::from_secs(179)).is_some());
        assert!(s.summary(Source::Codex, now + Duration::from_secs(180)).is_none());
    }
    #[test]
    fn datagrams_and_session_storage_are_bounded() {
        let mut event = Event { source: Source::Claude, event: Status::Done, session: "s".into(), project: "p".into() };
        event.session = "s".repeat(257); assert!(event.encode().is_none());
        event.session = "s\n".into(); assert!(event.encode().is_none());
        event.session = "s".into(); event.project = "한".repeat(101); assert!(event.encode().is_none());
        assert!(Event::decode(&vec![b' '; 4097]).is_none());
        let mut s = Sessions::default(); let now = Instant::now(); event.project = "p".into();
        for i in 0..300 { event.session = i.to_string(); s.apply(event.clone(), now); }
        assert_eq!(s.entries.len(), 256); assert_eq!(s.summary(Source::Claude, now).unwrap().more, 255);
        s.apply(Event { source: Source::Codex, session: "299".into(), ..event.clone() }, now);
        assert!(s.summary(Source::Claude, now).is_some()); assert!(s.summary(Source::Codex, now).is_some());
        assert!(s.summary(Source::Claude, now + Duration::from_secs(600)).is_none());
        s.apply(event, now + Duration::from_secs(600)); assert_eq!(s.entries.len(), 1);
    }
    #[test]
    fn udp_ignores_oversized_and_malformed_packets_then_accepts_valid_event() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let target = receiver.local_addr().unwrap();
        let mut buf = [0; 65535];
        for invalid in [vec![b' '; 10000], b"invalid JSON".to_vec()] {
            sender.send_to(&invalid, target).unwrap();
            assert!(receive(&receiver, &mut buf).unwrap().is_none());
        }
        let event = Event { source: Source::Codex, event: Status::Done, session: "s".into(), project: "p".into() };
        sender.send_to(&event.encode().unwrap(), target).unwrap();
        assert_eq!(receive(&receiver, &mut buf).unwrap(), Some(event));
    }
}
