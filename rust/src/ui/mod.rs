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
mod offline;
mod standby;
mod usage;
mod weather;

pub use canvas::Canvas;
pub use offline::{clip_base, clip_frame, offline_base, offline_frame, offline_len, offline_preview, Clip, Slide, CLIP_FADE_S};
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
use crate::state::{AiErr, State, AI_NAMES};
use crate::config;
use crate::i18n::{self, fill, TextFont};
use crate::sources::weather::{self as wx, Air, Weather};

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

const COL_A: (f32, f32) = (30.0, 596.0);
const COL_B: (f32, f32) = (640.0, 1184.0);
const COL_C: (f32, f32) = (1244.0, 1888.0);
const RULES_X: [f32; 2] = [610.0, 1214.0];
const TOP: f32 = 36.0;
const BOTTOM: f32 = 452.0;

/// Pitch of the CPU / memory / GPU / VRAM rows (leaves room for the network and disk row).
const HW_ROW: f32 = 62.0;
pub const GRAPH_W: f32 = 250.0;
const GRAPH_H: f32 = 47.0;
const GRAPH_STEP: f32 = 2.0;
pub const HIST_LEN: usize = (GRAPH_W / GRAPH_STEP) as usize + 1;


fn mix(c: Rgb, bg: Rgb, a: f32) -> Rgb {
    [0, 1, 2].map(|i| (c[i] as f32 * a + bg[i] as f32 * (1.0 - a)) as u8)
}

const AI_C1: f32 = 206.0;
const AI_C2: f32 = 354.0;
const AI_TOP: f32 = TOP;
const AI_ROW: f32 = 126.0;

// --- page --------------------------------------------------------------------------------

pub fn render(st: &State, now: DateTime<Local>) -> Canvas {
    let mut cv = Canvas::background();
    cv.rect(0.0, 0.0, 610.0, H as f32, [17, 25, 36]);
    cv.rect(610.0, 0.0, 1214.0, H as f32, [13, 20, 30]);
    cv.rect(1214.0, 0.0, W as f32, H as f32, [19, 27, 39]);
    let (ax, ar) = COL_A;
    let cal_x = ar - 210.0;
    draw_clock(&mut cv, &now, ax, cal_x - 24.0);
    draw_calendar(&mut cv, &now, cal_x, TOP, 210.0);
    cv.line(&[(ax, 262.0), (ar, 262.0)], 1.0, RULE);
    draw_weather(&mut cv, st.weather.as_ref(), ax, 284.0, ar);

    draw_hardware(&mut cv, st, COL_B.0, COL_B.1);
    draw_fans(&mut cv, st, COL_B.0, COL_B.1);
    draw_ai(&mut cv, st, COL_C.0, AI_TOP);

    // The jade and paper panels use flat backgrounds.
    /*cv.glow((ax - 20.0) as u32, 0, (cal_x - 10.0) as u32, 240); // clock
    let gx = COL_B.0 + 150.0;
    cv.glow((gx - 10.0) as u32, (TOP - 10.0) as u32, (gx + GRAPH_W + 10.0) as u32, (TOP + 3.0 * HW_ROW + GRAPH_H + 20.0) as u32);*/
    cv
}
