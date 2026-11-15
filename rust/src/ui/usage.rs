//! Column C: AI plan usage (what is left of the 5-hour and weekly limits).

use super::*;

/// What is left, in whole percent: nearest, as the CLIs show it, but anything used never reads as 100.
fn shown_left(used: f64) -> i64 {
    (100.0 - used).max(0.0).round().min(if used > 0.0 { 99.0 } else { 100.0 }) as i64
}

/// What the status line says (taken from the state when the dashboard is redrawn).
struct AgentStatus {
    waiting: bool,
    label: String,
    project: String,
}

/// Status for Claude, Codex and Antigravity: a session done or waiting for the
/// user, else "sign in again" while that service's login is gone.
fn agent_statuses(st: &State) -> [Option<AgentStatus>; 3] {
    use crate::agent_status::{Source, Status};
    let t = i18n::get();
    let ai = &config::get().ai;
    let now = std::time::Instant::now();
    [(Source::Claude, ai.claude, AI_NAMES[0]), (Source::Codex, ai.codex, AI_NAMES[1]), (Source::Antigravity, ai.antigravity, AI_NAMES[2])].map(|(src, on, name)| {
        if !on {
            return None;
        }
        let Some(s) = st.agents.summary(src, now) else {
            let signed_out = matches!(st.ai_err.get(name), Some(AiErr::SignedOut));
            return signed_out.then(|| AgentStatus { waiting: true, label: t.signed_out.into(), project: String::new() });
        };
        let waiting = s.status == Status::Waiting;
        let more = if s.more > 0 { format!(" +{}", s.more) } else { String::new() };
        Some(AgentStatus { waiting, label: format!("{}{more}", t.agent[if waiting { 0 } else { 1 }]), project: s.project.to_string() })
    })
}

fn draw_agent_status(cv: &mut Canvas, x: f32, y: f32, right: f32, status: &AgentStatus) {
    let color = if status.waiting { [222, 173, 118] } else { [139, 202, 166] };
    let font = ko(14.0, false);
    let mut project = status.project.clone();
    if cv.text_len(&project, font) > right - x - 16.0 {
        while !project.is_empty() && cv.text_len(&format!("{project}…"), font) > right - x - 16.0 {
            project.pop();
        }
        project.push('…');
    }
    let label_font = fit_ko(cv, &status.label, &[17.0, 16.0, 15.0, 14.0], true, right - x - 16.0);
    let width = cv.text_len(&status.label, label_font).max(cv.text_len(&project, font)) + 16.0;
    let height = if project.is_empty() { 25.0 } else { 44.0 };
    let outline = [
        (x, y), (x + width, y), (x + width, y + height), (x, y + height),
        (x, y + 22.0), (x - 8.0, y + 17.0), (x, y + 12.0), (x, y),
    ];
    cv.polygon(&outline, [29, 41, 58]);
    cv.line(&outline, 1.0, color);
    cv.text(x + 8.0, y + 3.0, &status.label, label_font, color);
    if !project.is_empty() {
        cv.text(x + 8.0, y + 24.0, &project, font, [165, 179, 198]);
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
    [0, 1, 2].map(|i| crate::mascot::Area { x0: x + 28.0, x1: x + 50.0, ground: AI_TOP + 34.0 + i as f32 * AI_ROW + 109.0 })
}

pub(super) fn draw_ai(cv: &mut Canvas, st: &State, x: f32, top: f32) {
    let (c1, c2) = (x + AI_C1, x + AI_C2);
    let c3 = c2 + (c2 - c1);
    let width = (COL_C.1 - c3).min(c2 - c1 - 16.0);
    let statuses = agent_statuses(st);
    let f_val = num(32.0, Face::Num);
    cv.caps(x, top, "AI Left", 16.0, Some([222, 173, 118]), 2.5, false);
    cv.caps(c1, top, "5 Hours", 14.0, Some([137, 155, 179]), 2.0, false);
    cv.caps(c2, top, "Weekly", 14.0, Some([137, 155, 179]), 2.0, false);
    let ai = &config::get().ai;
    let enabled = [ai.claude, ai.codex, ai.antigravity];
    if AI_NAMES.iter().zip(enabled).any(|(name, on)| on && st.ai.get(name).is_some_and(|u| u.weekly_extra.is_some())) {
        cv.caps(c3, top, "Model Weekly", 14.0, Some([137, 155, 179]), 1.0, false);
    }
    for (i, name) in AI_NAMES.iter().enumerate() {
        if ![ai.claude, ai.codex, ai.antigravity][i] {
            continue; // turned off in the settings
        }
        let y = top + 34.0 + i as f32 * AI_ROW;
        cv.line(&[(x, y - 5.0), (COL_C.1, y - 5.0)], 1.0, [45, 58, 77]);
        let color = ai_color(name);
        cv.rect(x, y + 10.0, x + 4.0, y + 30.0, color); // brand tick
        let f_name = num(24.0, Face::NumSemi);
        let display = if *name == "Antigravity" { "Gemini" } else { name };
        cv.text(x + 16.0, y + 5.0, display, f_name, [232, 238, 246]);
        let u = st.ai.get(name);
        // plan name after the service name, smaller until it fits before the first column
        let mut px = x + 16.0 + cv.text_len(display, f_name) + 8.0;
        if let Some(plan) = u.and_then(|u| crate::sources::ai::plan_label(name, &u.plan)) {
            if let Some(f) = [14.0, 13.0, 12.0, 11.0].map(|s| num(s, Face::NumSemi)).into_iter().find(|&f| px + cv.text_len(&plan, f) <= c1 - 10.0) {
                cv.text(px, y + 15.0, &plan, f, [137, 155, 179]);
                px += cv.text_len(&plan, f) + 8.0;
            }
        }
        // Codex refill coupons: a ticket and how many
        if let Some(n) = u.map(|u| u.coupons).filter(|&n| n > 0) {
            draw_coupons(cv, px, y + 16.0, n, color, c1 - 10.0);
        }

        if let Some(status) = &statuses[i] {
            draw_agent_status(cv, x + 83.0, y + 66.0, c1 - 12.0, status);
        }
        let err = st.ai_err.get(name).filter(|e| !matches!(e, AiErr::SignedOut));
        if let (Some(e), None) = (err, u) {
            cv.text(c1, y + 8.0, &err_text(e), ko(18.0, false), [222, 173, 118]);
            continue;
        }
        if let Some(e) = err {
            // value is stale; say why under the name
            let t = err_text(e);
            cv.text(x + 16.0, y + 39.0, t.split(" · ").next().unwrap_or(&t), ko(15.0, false), [222, 173, 118]);
        } else if let Some(c) = st.api_cost.as_ref().filter(|_| *name == "Antigravity") {
            // Gemini API spend this month, a line under the name (the mascot walks over it)
            let text = format!("Gemini API {}", crate::sources::api_cost::format(c.current(), &c.currency));
            cv.text(x + 16.0, y + 39.0, &text, ko(15.0, false), [137, 155, 179]);
        }
        let cells = match u {
            Some(u) => [(u.session, u.session_reset, None), (u.weekly, u.weekly_reset, u.weekly_extra)],
            None => [(None, None, None), (None, None, None)],
        };
        for (cx, (pct, reset, extra)) in [c1, c2].into_iter().zip(cells) {
            let refilled = reset.is_some_and(|r| r <= epoch_now());
            let pct = pct.map(|used| if refilled { 0.0 } else { used });
            if let Some(used) = pct {
                let val = format!("{}%", shown_left(used));
                cv.text(cx, y + 4.0, &val, f_val, if used >= 90.0 { [227, 133, 108] } else { [232, 238, 246] });
            } else {
                cv.text(cx, y + 4.0, "--", f_val, [100, 132, 118]);
            }
            remaining_bar(cv, cx, y + 46.0, width, 9.0, pct.unwrap_or(100.0), color, false);
            if let Some(used) = extra {
                let label = if *name == "Claude" { "Fable" } else { "Argon" };
                cv.text(c3, y + 4.0, &format!("{}%", shown_left(used)), f_val, if used >= 90.0 { [227, 133, 108] } else { [165, 179, 198] });
                remaining_bar(cv, c3, y + 46.0, width, 9.0, used, color, used >= 100.0);
                cv.text(c3, y + 63.0, label, num(18.0, Face::NumSemi), [232, 238, 246]);
                if used >= 100.0 {
                    let label = "LIMIT HIT";
                    let font = ko(15.0, true);
                    let x = c3 + cv.text_len("0%", f_val) + 9.0;
                    let top = y + 12.0;
                    cv.rect(x, top, x + cv.text_len(label, font) + 12.0, top + 23.0, color);
                    cv.text(x + 6.0, top + 2.0, label, font, [19, 27, 39]);
                }
            }
            if let Some(reset) = reset {
                draw_reset_time(cv, cx, y + 85.0, reset - epoch_now());
            }
        }
    }
}

fn draw_reset_time(cv: &mut Canvas, mut x: f32, y: f32, seconds: f64) {
    let minutes = (seconds.max(0.0) / 60.0).ceil() as u64;
    for (amount, unit) in [(minutes / 1440, "d"), (minutes / 60 % 24, "h"), (minutes % 60, "m")] {
        if amount == 0 && (unit != "m" || minutes != 0) {
            continue;
        }
        let text = amount.to_string();
        let number = num(25.0, Face::NumSemi);
        let suffix = num(15.0, Face::Num);
        cv.text(x, y, &text, number, INK);
        x += cv.text_len(&text, number) + 1.0;
        cv.text(x, y + 9.0, unit, suffix, [165, 179, 198]);
        x += cv.text_len(unit, suffix) + 3.0;
    }
}

fn remaining_bar(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, used: f64, color: Rgb, exhausted: bool) {
    let (blocks, gap) = (10, 3.0);
    let block_w = (w - gap * (blocks - 1) as f32) / blocks as f32;
    let left = ((100.0 - used).clamp(0.0, 100.0) / 10.0) as f32;
    for i in 0..blocks {
        let bx = x + i as f32 * (block_w + gap);
        cv.rect(bx, y, bx + block_w, y + h, [45, 58, 77]);
        if exhausted {
            let edge = mix(color, [19, 27, 39], 0.72);
            cv.rect(bx, y, bx + block_w, y + h, edge);
            cv.rect(bx + 1.0, y + 1.0, bx + block_w - 1.0, y + h - 1.0, [19, 27, 39]);
        }
        let part = (left - i as f32).clamp(0.0, 1.0);
        if part > 0.0 {
            let fill = (h * part).max(2.0);
            cv.rect(bx, y + h - fill, bx + block_w, y + h, color);
        }
    }
}
