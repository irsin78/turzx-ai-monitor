//! Number, rate and time formatting.

use super::*;

pub(super) fn temp_color(t: f64) -> Rgb {
    if t < 70.0 { INK } else if t < 85.0 { WARN } else { HOT }
}

/// The largest of `sizes` at which `s` fits in `max_w` (translations differ a lot in length).
pub(super) fn fit_ko(cv: &Canvas, s: &str, sizes: &[f32], bold: bool, max_w: f32) -> Font_ {
    sizes.iter().map(|&z| ko(z, bold)).find(|&f| cv.text_len(s, f) <= max_w).unwrap_or(ko(*sizes.last().unwrap(), bold))
}

/// Temperature in the unit from the settings (sensors and weather report °C).
pub(super) fn deg(c: f64) -> f64 {
    if config::get().fahrenheit() { c * 9.0 / 5.0 + 32.0 } else { c }
}

pub(super) fn fmt_left(reset: Option<f64>) -> String {
    reset.map_or(String::new(), |r| i18n::get().time_left((r - epoch_now()) as i64))
}

pub(super) fn err_text(e: &AiErr) -> String {
    let t = i18n::get();
    match e {
        AiErr::RateLimited(until) if *until > epoch_now() + 30.0 => fill(t.rate_limited, fmt_left(Some(*until))),
        AiErr::RateLimited(_) => t.rate_limited_soon.into(),
        AiErr::Http(401) => t.signed_out.into(),
        AiErr::Http(c) => fill(t.failed, c),
        AiErr::Other(s) => fill(t.failed, s),
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
