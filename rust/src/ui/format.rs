//! Number, rate and time formatting.

use super::*;

pub(super) fn temp_color(t: f64) -> Rgb {
    if t < 70.0 { INK } else if t < 85.0 { WARN } else { HOT }
}

pub(super) fn fmt_left(reset: Option<f64>) -> String {
    let Some(reset) = reset else { return String::new() };
    let s = (reset - epoch_now()).max(0.0) as i64;
    let (d, h, m) = (s / 86400, s % 86400 / 3600, s % 3600 / 60);
    if d > 0 { format!("{d}일 {h}시간") } else if h > 0 { format!("{h}시간 {m}분") } else { format!("{m}분") }
}

pub(super) fn err_text(e: &AiErr) -> String {
    match e {
        AiErr::RateLimited(until) => {
            let left = fmt_left(Some(*until));
            format!("요청 제한 · {} 후 재시도", if left.is_empty() { "곧".into() } else { left })
        }
        AiErr::Http(401) => "로그인 만료 · Claude Code 실행 필요".into(),
        AiErr::Http(c) => format!("조회 실패 ({c})"),
        AiErr::Other(s) => format!("조회 실패 ({s})"),
    }
}

/// Network rate in bits/s, as Task Manager shows it.
pub(super) fn fmt_rate(bytes_per_s: f64) -> (String, &'static str) {
    let bits = bytes_per_s * 8.0;
    for (unit, scale) in [("Gbps", 1e9), ("Mbps", 1e6), ("Kbps", 1e3)] {
        if bits >= scale {
            let v = bits / scale;
            return (if v < 100.0 { format!("{v:.1}") } else { format!("{v:.0}") }, unit);
        }
    }
    (format!("{bits:.0}"), "bps")
}

pub(super) fn thousands(v: f64) -> String {
    let s = format!("{:.0}", v);
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
