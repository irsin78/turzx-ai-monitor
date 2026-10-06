//! Dashboard rendering (1920x480, landscape).
//!
//! Look: no cards, hairline rules between three columns, one type color plus a gray for
//! labels; color only for meaning (heat, Sundays/holidays, AI brand colors). Theme "night":
//! navy gradient with a soft glow on the clock and graphs. Fonts: HarmonyOS Sans (digits,
//! Latin) and Noto Sans KR (text), embedded.
//!
//! Each widget lives in its own file and draws into a [`Canvas`]; `render` lays them out.
#![allow(unused_imports)] // shared imports for the widget files (`use super::*`)

mod canvas;
mod clock;
mod format;
mod hardware;
mod icons;
mod standby;
mod usage;
mod weather;

pub use canvas::Canvas;
pub use standby::{standby, standby_page};
pub use usage::mascot_areas;

use canvas::*;
use clock::*;
use format::*;
use hardware::*;
use icons::*;
use usage::*;
use weather::*;

use std::collections::VecDeque;
use std::sync::OnceLock;

use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont, VariableFont};
use chrono::{DateTime, Datelike, Local, Timelike};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use crate::sources::ai::now as epoch_now;
use crate::sources::holidays::is_holiday;
use crate::state::{AiErr, State, AI_NAMES};
use crate::sources::weather::{air_grade, Weather};

pub const W: u32 = 1920;
pub const H: u32 = 480;

type Rgb = [u8; 3];

// Theme "night"
const INK: Rgb = [232, 238, 252];
const MUTED: Rgb = [122, 136, 168];
const SOFT: Rgb = [186, 198, 224];
const FAINT: Rgb = [58, 70, 98];
const RULE: Rgb = [34, 44, 70];
const BG: Rgb = [10, 14, 26];
const BG_FILL: Rgb = [20, 28, 48];
const BG_LINE: Rgb = [50, 64, 96];
const CORE_BOX: Rgb = [36, 46, 72];
const WARN: Rgb = [232, 168, 72];
const HOT: Rgb = [236, 92, 70];
const SUNDAY: Rgb = [240, 120, 120];
const SATURDAY: Rgb = [120, 170, 255];
const ACCENT: Rgb = [170, 200, 255];
const LABEL: Rgb = [120, 150, 210];
const GRADIENT: (Rgb, Rgb) = ([20, 30, 58], [5, 7, 13]);
const GLOW: (Rgb, f32) = ([90, 140, 255], 0.55);

fn ai_color(name: &str) -> Rgb {
    match name {
        "Claude" => [217, 119, 87],
        "Codex" => [235, 238, 245],
        _ => [66, 133, 244], // Antigravity
    }
}

const COL_A: (f32, f32) = (40.0, 640.0);
const COL_B: (f32, f32) = (712.0, 1252.0);
const COL_C: (f32, f32) = (1324.0, 1880.0);
const RULES_X: [f32; 2] = [676.0, 1288.0];
const TOP: f32 = 36.0;
const BOTTOM: f32 = 444.0;

pub const GRAPH_W: f32 = 250.0;
const GRAPH_H: f32 = 56.0;
const GRAPH_STEP: f32 = 2.0;
pub const HIST_LEN: usize = (GRAPH_W / GRAPH_STEP) as usize + 1;

const WEEKDAYS: [&str; 7] = ["월", "화", "수", "목", "금", "토", "일"];

fn mix(c: Rgb, bg: Rgb, a: f32) -> Rgb {
    [0, 1, 2].map(|i| (c[i] as f32 * a + bg[i] as f32 * (1.0 - a)) as u8)
}

const AI_C1: f32 = 196.0;
const AI_C2: f32 = 382.0;
const AI_TOP: f32 = 150.0;
const AI_ROW: f32 = 90.0;

// --- page --------------------------------------------------------------------------------

pub fn render(st: &State, now: DateTime<Local>) -> Canvas {
    let mut cv = Canvas::background();
    for rx in RULES_X {
        cv.line(&[(rx, TOP), (rx, BOTTOM)], 1.0, RULE);
    }
    let (ax, ar) = COL_A;
    let cal_x = ar - 212.0;
    draw_clock(&mut cv, &now, ax, cal_x - 24.0);
    draw_calendar(&mut cv, &now, cal_x, TOP, 212.0);
    cv.line(&[(ax, 262.0), (ar, 262.0)], 1.0, RULE);
    draw_weather(&mut cv, st, ax, 284.0, ar);

    draw_hardware(&mut cv, st, COL_B.0, COL_B.1);
    let (cx, cr) = COL_C;
    draw_fans(&mut cv, st, cx, cr);
    cv.line(&[(cx, 130.0), (cr, 130.0)], 1.0, RULE);
    draw_ai(&mut cv, st, cx, AI_TOP);

    cv.glow((ax - 20.0) as u32, 0, (cal_x - 10.0) as u32, 240); // clock
    let gx = COL_B.0 + 150.0;
    cv.glow((gx - 10.0) as u32, (TOP - 10.0) as u32, (gx + GRAPH_W + 10.0) as u32, (TOP + 3.0 * 84.0 + GRAPH_H + 20.0) as u32);
    cv
}
