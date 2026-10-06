//! Column C: AI plan usage (what is left of the 5-hour and weekly limits).

use super::*;

/// Blocky bar (Minecraft style): each block is 100/segments %, the last one fills partially.
pub(super) fn segmented_bar(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, pct: f64, color: Rgb) {
    let (segments, gap) = (10usize, 3.0);
    let seg_w = (w - gap * (segments - 1) as f32) / segments as f32;
    let filled = (pct.clamp(0.0, 100.0) / 100.0 * segments as f64) as f32;
    for i in 0..segments {
        let sx = x + i as f32 * (seg_w + gap);
        cv.rect(sx, y, sx + seg_w - 1.0, y + h - 1.0, RULE);
        let part = (filled - i as f32).clamp(0.0, 1.0);
        if part > 0.0 {
            cv.rect(sx, y, sx + (seg_w * part).max(2.0) - 1.0, y + h - 1.0, color);
            if part >= 1.0 {
                // light top edge, like a pixel-art block
                cv.line(&[(sx, y), (sx + seg_w - 1.0, y)], 2.0, mix([255, 255, 255], color, 0.35));
            }
        }
    }
}

/// Strips under the AI names where the mascots roam (Claude, Codex, Antigravity).
pub fn mascot_areas() -> [crate::mascot::Area; 3] {
    let x = COL_C.0;
    [0, 1, 2].map(|i| crate::mascot::Area { x0: x + 30.0, x1: x + AI_C1 - 30.0, ground: AI_TOP + 34.0 + i as f32 * AI_ROW + 86.0 })
}

pub(super) fn draw_ai(cv: &mut Canvas, st: &State, x: f32, top: f32) {
    let (c1, c2) = (x + AI_C1, x + AI_C2);
    let bw = 150.0;
    let f_val = num(32.0, Face::Num);
    cv.caps(x, top, "AI Left", 16.0, None, 2.5, false);
    cv.caps(c1, top, "5 Hours", 14.0, Some(MUTED), 2.0, false);
    cv.caps(c2, top, "Weekly", 14.0, Some(MUTED), 2.0, false);
    let ai = &config::get().ai;
    for (i, name) in AI_NAMES.iter().enumerate() {
        if ![ai.claude, ai.codex, ai.antigravity][i] {
            continue; // turned off in the settings
        }
        let y = top + 34.0 + i as f32 * AI_ROW;
        let color = ai_color(name);
        cv.rect(x, y + 10.0, x + 4.0, y + 30.0, color); // brand tick
        cv.text(x + 16.0, y + 5.0, name, num(24.0, Face::NumSemi), INK);
        let u = st.ai.get(name);
        let err = st.ai_err.get(name);
        if let (Some(e), None) = (err, u) {
            cv.text(c1, y + 8.0, &err_text(e), ko(18.0, false), WARN);
            continue;
        }
        if let Some(e) = err {
            // value is stale; say why under the name
            let t = err_text(e);
            cv.text(x + 16.0, y + 38.0, t.split(" · ").next().unwrap_or(&t), ko(15.0, false), WARN);
        }
        let cells = match u {
            Some(u) => [(u.session, u.session_reset), (u.weekly, u.weekly_reset)],
            None => [(None, None), (None, None)],
        };
        for (cx, (pct, reset)) in [c1, c2].into_iter().zip(cells) {
            let Some(pct) = pct else {
                segmented_bar(cv, cx, y + 40.0, bw, 12.0, 0.0, color);
                cv.text(cx, y, "--", f_val, FAINT);
                continue;
            };
            // reset time passed: the limit has refilled; show that until the next read confirms it
            let refilled = reset.is_some_and(|r| r <= epoch_now());
            let pct = if refilled { 0.0 } else { pct };
            let left = (100.0 - pct).max(0.0); // show what is left, not what is used
            let val = ((left + 1e-9).floor() as i64).to_string(); // round down: 99.99 must not read as 100
            cv.text(cx, y, &val, f_val, if left <= 10.0 { HOT } else { INK });
            cv.text(cx + cv.text_len(&val, f_val) + 4.0, y + 12.0, "%", num(16.0, Face::NumSemi), MUTED);
            segmented_bar(cv, cx, y + 40.0, bw, 12.0, left, color);
            let when = if refilled { i18n::get().refilled.to_string() } else { fmt_left(reset) };
            cv.text(cx, y + 58.0, &when, ko(16.0, false), SOFT);
        }
    }
}
