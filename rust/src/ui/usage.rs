//! Column C: AI plan usage (what is left of the 5-hour and weekly limits).

use super::*;

/// Blocky bar (Minecraft style): each block is 100/segments %, the last one fills partially.
pub(super) fn segmented_bar(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, pct: f64, color: Rgb) {
    segmented_fill(cv, x, y, w, h, 100.0, RULE, false);
    segmented_fill(cv, x, y, w, h, pct, color, true);
}

/// The filled blocks of `segmented_bar` only (so a second bar can go on top of the first).
fn segmented_fill(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, pct: f64, color: Rgb, edge: bool) {
    let (segments, gap) = (10usize, 3.0);
    let seg_w = (w - gap * (segments - 1) as f32) / segments as f32;
    let filled = (pct.clamp(0.0, 100.0) / 100.0 * segments as f64) as f32;
    for i in 0..segments {
        let sx = x + i as f32 * (seg_w + gap);
        let part = (filled - i as f32).clamp(0.0, 1.0);
        if part > 0.0 {
            cv.rect(sx, y, sx + (seg_w * part).max(2.0) - 1.0, y + h - 1.0, color);
            if edge && part >= 1.0 {
                // light top edge, like a pixel-art block
                cv.line(&[(sx, y), (sx + seg_w - 1.0, y)], 2.0, mix([255, 255, 255], color, 0.35));
            }
        }
    }
}

/// What is left, in whole percent: nearest, as the CLIs show it, but anything used never reads as 100.
fn shown_left(used: f64) -> i64 {
    (100.0 - used).max(0.0).round().min(if used > 0.0 { 99.0 } else { 100.0 }) as i64
}

/// What a status bubble says (taken from the state when the dashboard is redrawn).
pub struct Bubble {
    waiting: bool,
    label: String,
    project: String,
}

/// The bubbles for the Claude, Codex and Antigravity mascots: a session done or waiting for the user.
pub fn agent_bubbles(st: &State) -> [Option<Bubble>; 3] {
    use crate::agent_status::{Source, Status};
    let t = i18n::get();
    let ai = &config::get().ai;
    let now = std::time::Instant::now();
    [(Source::Claude, ai.claude), (Source::Codex, ai.codex), (Source::Antigravity, ai.antigravity)].map(|(src, on)| {
        let s = st.agents.summary(src, now).filter(|_| on)?;
        let waiting = s.status == Status::Waiting;
        let more = if s.more > 0 { format!(" +{}", s.more) } else { String::new() };
        Some(Bubble { waiting, label: format!("{}{more}", t.agent[if waiting { 0 } else { 1 }]), project: s.project.to_string() })
    })
}

/// A small bubble right above a mascot's head (`head`: its top middle), its tail pointing down
/// at the mascot; kept between `left` and `right`.
pub fn draw_agent_bubble(cv: &mut Canvas, head: (f32, f32), b: &Bubble, left: f32, right: f32) {
    let color = if b.waiting { WARN } else { [125, 221, 180] };
    let fill = [22, 32, 54];
    let (fl, fp) = (ko(12.0, true), ko(12.0, false));
    let mut project = b.project.clone();
    while !project.is_empty() && cv.text_len(&project, fp) > 150.0 {
        project.pop();
    }
    let w = cv.text_len(&b.label, fl).max(cv.text_len(&project, fp)) + 14.0;
    let h = if project.is_empty() { 18.0 } else { 32.0 };
    let tail = 7.0;
    let bottom = head.1 - 3.0 - tail;
    let top = bottom - h;
    let x = (head.0 - w / 2.0).clamp(left, (right - w).max(left));
    let tx = head.0.clamp(x + 8.0, x + w - 8.0);
    let c = 3.0; // cut corners
    let outline = [
        (x + c, top), (x + w - c, top), (x + w, top + c), (x + w, bottom - c), (x + w - c, bottom),
        (tx + 5.0, bottom), (tx, bottom + tail), (tx - 5.0, bottom),
        (x + c, bottom), (x, bottom - c), (x, top + c), (x + c, top),
    ];
    cv.polygon(&outline, fill);
    cv.line(&outline, 1.5, color);
    cv.text(x + 7.0, top + 1.0, &b.label, fl, color);
    if !project.is_empty() {
        cv.text(x + 7.0, top + 15.0, &project, fp, INK);
    }
}

/// An outlined ticket (notched sides, perforation) and `count` after it, if it fits before `right`.
fn draw_coupons(cv: &mut Canvas, x: f32, y: f32, count: u64, color: Rgb, right: f32) {
    let f = num(14.0, Face::NumSemi);
    let label = count.to_string();
    if x + 22.0 + cv.text_len(&label, f) > right {
        return;
    }
    cv.line(
        &[(x, y + 2.0), (x + 17.0, y + 2.0), (x + 17.0, y + 5.0), (x + 15.0, y + 7.0), (x + 17.0, y + 9.0), (x + 17.0, y + 12.0),
          (x, y + 12.0), (x, y + 9.0), (x + 2.0, y + 7.0), (x, y + 5.0), (x, y + 2.0)],
        1.5,
        color,
    );
    for dy in [4.0, 7.0, 10.0] {
        cv.rect(x + 11.0, y + dy, x + 12.0, y + dy + 1.0, color);
    }
    cv.text(x + 22.0, y - 1.0, &label, f, color);
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
        let f_name = num(24.0, Face::NumSemi);
        cv.text(x + 16.0, y + 5.0, name, f_name, INK);
        let u = st.ai.get(name);
        // plan name after the service name, smaller until it fits before the first column
        let mut px = x + 16.0 + cv.text_len(name, f_name) + 8.0;
        if let Some(plan) = u.and_then(|u| crate::sources::ai::plan_label(name, &u.plan)) {
            if let Some(f) = [14.0, 13.0, 12.0, 11.0].map(|s| num(s, Face::NumSemi)).into_iter().find(|&f| px + cv.text_len(&plan, f) <= c1 - 10.0) {
                cv.text(px, y + 15.0, &plan, f, MUTED);
                px += cv.text_len(&plan, f) + 8.0;
            }
        }
        // Codex refill coupons: a ticket and how many
        if let Some(n) = u.map(|u| u.coupons).filter(|&n| n > 0) {
            draw_coupons(cv, px, y + 16.0, n, color, c1 - 10.0);
        }

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
            Some(u) => [(u.session, u.session_reset, None), (u.weekly, u.weekly_reset, u.weekly_extra)],
            None => [(None, None, None), (None, None, None)],
        };
        for (cx, (pct, reset, extra)) in [c1, c2].into_iter().zip(cells) {
            let Some(pct) = pct else {
                segmented_bar(cv, cx, y + 40.0, bw, 12.0, 0.0, color);
                cv.text(cx, y, "--", f_val, FAINT);
                continue;
            };
            // reset time passed: the limit has refilled; show that until the next read confirms it
            let refilled = reset.is_some_and(|r| r <= epoch_now());
            let pct = if refilled { 0.0 } else { pct };
            let left = (100.0 - pct).max(0.0); // show what is left, not what is used
            let val = shown_left(pct).to_string();
            cv.text(cx, y, &val, f_val, if left <= 10.0 { HOT } else { INK });
            let f_pct = num(16.0, Face::NumSemi);
            let px = cx + cv.text_len(&val, f_val) + 4.0;
            cv.text(px, y + 12.0, "%", f_pct, MUTED);
            segmented_bar(cv, cx, y + 40.0, bw, 12.0, left, color);
            // a second limit in the same window (Argon): "/90%" smaller, its bar the same blocks
            // in a darker shade; the shorter of the two goes on top so both show
            if let Some(e) = extra {
                let e_left = (100.0 - e).max(0.0);
                let f_extra = num(20.0, Face::NumSemi);
                let label = format!("/{}", shown_left(e));
                let ex = px + cv.text_len("%", f_pct) + 4.0;
                cv.text(ex, y + 8.0, &label, f_extra, if e_left <= 10.0 { HOT } else { SOFT });
                cv.text(ex + cv.text_len(&label, f_extra) + 3.0, y + 12.0, "%", f_pct, MUTED);
                let dark = mix(color, [0, 0, 0], 0.6);
                let mut bars = [(left, color), (e_left, dark)];
                bars.sort_by(|a, b| b.0.total_cmp(&a.0));
                for (pct, c) in bars {
                    segmented_fill(cv, cx, y + 40.0, bw, 12.0, pct, c, true);
                }
            }
            let when = if refilled { i18n::get().refilled.to_string() } else { fmt_left(reset) };
            cv.text(cx, y + 58.0, &when, ko(16.0, false), SOFT);
        }
    }
}
