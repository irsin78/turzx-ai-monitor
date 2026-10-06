//! Dashboard rendering (1920x480, landscape).
//!
//! Look: no cards, hairline rules between three columns, one type color plus a gray for
//! labels; color only for meaning (heat, Sundays/holidays, AI brand colors). Theme "night":
//! navy gradient with a soft glow on the clock and graphs. Fonts: HarmonyOS Sans (digits,
//! Latin) and Noto Sans KR (Hangul), embedded. Coordinates and anchors mirror the Python
//! version (PIL), so both render the same layout.

use std::collections::VecDeque;
use std::sync::OnceLock;

use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont, VariableFont};
use chrono::{DateTime, Datelike, Local, Timelike};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use crate::ai::now as epoch_now;
use crate::holidays_kr::is_holiday;
use crate::state::{AiErr, State, AI_NAMES};
use crate::weather::{air_grade, Weather};

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

// --- fonts --------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub enum Face {
    NumLight,
    Num,
    NumSemi,
    Ko,
    KoBold,
}

struct Fonts {
    num_light: FontRef<'static>,
    num: FontRef<'static>,
    num_semi: FontRef<'static>,
    ko: FontRef<'static>,
    ko_bold: FontRef<'static>,
}

static HARMONY_THIN: &[u8] = include_bytes!("../../fonts/HarmonyOS_Sans_Thin.ttf");
static HARMONY_LIGHT: &[u8] = include_bytes!("../../fonts/HarmonyOS_Sans_Light.ttf");
static HARMONY_MEDIUM: &[u8] = include_bytes!("../../fonts/HarmonyOS_Sans_Medium.ttf");
static NOTO_KR: &[u8] = include_bytes!("../../fonts/NotoSansKR-VF.ttf");
const NUM_SCALE: f32 = 0.92; // sizes are tuned for Bahnschrift; evens out HarmonyOS metrics

fn fonts() -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| {
        let noto = |wght: f32| {
            let mut f = FontRef::try_from_slice(NOTO_KR).expect("Noto Sans KR");
            f.set_variation(b"wght", wght);
            f
        };
        Fonts {
            num_light: FontRef::try_from_slice(HARMONY_THIN).expect("HarmonyOS Thin"),
            num: FontRef::try_from_slice(HARMONY_LIGHT).expect("HarmonyOS Light"),
            num_semi: FontRef::try_from_slice(HARMONY_MEDIUM).expect("HarmonyOS Medium"),
            ko: noto(300.0),      // Light
            ko_bold: noto(500.0), // Medium
        }
    })
}

/// A font face at a PIL-style size (pixels per em).
#[derive(Clone, Copy)]
pub struct Font_ {
    face: Face,
    size: f32,
}

fn num(size: f32, face: Face) -> Font_ {
    Font_ { face, size: (size * NUM_SCALE).round() }
}

fn ko(size: f32, bold: bool) -> Font_ {
    Font_ { face: if bold { Face::KoBold } else { Face::Ko }, size }
}

impl Font_ {
    fn font(&self) -> &'static FontRef<'static> {
        let f = fonts();
        match self.face {
            Face::NumLight => &f.num_light,
            Face::Num => &f.num,
            Face::NumSemi => &f.num_semi,
            Face::Ko => &f.ko,
            Face::KoBold => &f.ko_bold,
        }
    }

    fn scaled(&self) -> ab_glyph::PxScaleFont<&'static FontRef<'static>> {
        let f = self.font();
        let em = f.units_per_em().unwrap_or(1000.0);
        f.as_scaled(PxScale::from(self.size * f.height_unscaled() / em))
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Anchor {
    La, // left, ascender (PIL default)
    Ls, // left, baseline
    Mt, // middle, top of ink
    Mm, // middle, middle of ascender/descender
    Ra, // right, ascender
}

// --- canvas -------------------------------------------------------------------------

#[derive(Clone)]
pub struct Canvas {
    pm: Pixmap,
}

fn paint(c: Rgb) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], 255);
    p.anti_alias = true;
    p
}

impl Canvas {
    fn background() -> Canvas {
        static BGPM: OnceLock<Pixmap> = OnceLock::new();
        let pm = BGPM.get_or_init(|| {
            // PIL: mask = 0.4 * (top->bottom) + 0.6 * (left 255 -> right 0); mask picks color #2
            let mut pm = Pixmap::new(W, H).unwrap();
            let (c0, c1) = GRADIENT;
            for (i, px) in pm.pixels_mut().iter_mut().enumerate() {
                let (x, y) = ((i as u32 % W) as f32, (i as u32 / W) as f32);
                let v = y / (H - 1) as f32 * 255.0;
                let h = 255.0 - x / (W - 1) as f32 * 255.0;
                let m = (0.4 * v + 0.6 * h) / 255.0;
                let c = [0, 1, 2].map(|k| (c1[k] as f32 * m + c0[k] as f32 * (1.0 - m)) as u8);
                *px = tiny_skia::PremultipliedColorU8::from_rgba(c[0], c[1], c[2], 255).unwrap();
            }
            pm
        });
        Canvas { pm: pm.clone() }
    }

    fn get(&self, x: f32, y: f32) -> Rgb {
        let (x, y) = (x.clamp(0.0, W as f32 - 1.0) as u32, y.clamp(0.0, H as f32 - 1.0) as u32);
        let p = self.pm.pixels()[(y * W + x) as usize];
        [p.red(), p.green(), p.blue()]
    }

    fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let px = &mut self.pm.pixels_mut()[(y as u32 * W + x as u32) as usize];
        let old = [px.red(), px.green(), px.blue()];
        let n = [0, 1, 2].map(|i| (old[i] as f32 * (1.0 - a) + c[i] as f32 * a).round() as u8);
        *px = tiny_skia::PremultipliedColorU8::from_rgba(n[0], n[1], n[2], 255).unwrap();
    }

    /// PIL rectangle: inclusive corners
    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgb) {
        if let Some(r) = Rect::from_ltrb(x0, y0, x1 + 1.0, y1 + 1.0) {
            let mut p = paint(c);
            p.anti_alias = false;
            self.pm.fill_rect(r, &p, Transform::identity(), None);
        }
    }

    fn ellipse(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgb) {
        if let Some(r) = Rect::from_ltrb(x0, y0, x1 + 1.0, y1 + 1.0) {
            if let Some(path) = PathBuilder::from_oval(r) {
                self.pm.fill_path(&path, &paint(c), FillRule::Winding, Transform::identity(), None);
            }
        }
    }

    fn polygon(&mut self, pts: &[(f32, f32)], c: Rgb) {
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0, pts[0].1);
        for p in &pts[1..] {
            pb.line_to(p.0, p.1);
        }
        pb.close();
        if let Some(path) = pb.finish() {
            self.pm.fill_path(&path, &paint(c), FillRule::Winding, Transform::identity(), None);
        }
    }

    fn polygon_alpha(&mut self, pts: &[(f32, f32)], c: Rgb, a: f32) {
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0, pts[0].1);
        for p in &pts[1..] {
            pb.line_to(p.0, p.1);
        }
        pb.close();
        if let Some(path) = pb.finish() {
            let mut p = paint(c);
            p.set_color_rgba8(c[0], c[1], c[2], (a * 255.0) as u8);
            self.pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
        }
    }

    /// Polyline; 1-px lines on integer coordinates are nudged to pixel centers like PIL.
    fn line(&mut self, pts: &[(f32, f32)], width: f32, c: Rgb) {
        let off = if width <= 1.0 { 0.5 } else { 0.0 };
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0 + off, pts[0].1 + off);
        for p in &pts[1..] {
            pb.line_to(p.0 + off, p.1 + off);
        }
        if let Some(path) = pb.finish() {
            let stroke = Stroke { width, line_join: LineJoin::Round, line_cap: LineCap::Butt, ..Default::default() };
            self.pm.stroke_path(&path, &paint(c), &stroke, Transform::identity(), None);
        }
    }

    fn point(&mut self, x: f32, y: f32, c: Rgb) {
        self.blend(x as i32, y as i32, c, 1.0);
    }

    fn text_len(&self, s: &str, f: Font_) -> f32 {
        let sf = f.scaled();
        let mut w = 0.0;
        let mut prev = None;
        for ch in s.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                w += sf.kern(p, id);
            }
            w += sf.h_advance(id);
            prev = Some(id);
        }
        w
    }

    /// Ink top of `s` relative to its baseline (negative = above).
    fn ink_top(&self, s: &str, f: Font_) -> f32 {
        let sf = f.scaled();
        s.chars()
            .filter_map(|ch| sf.outline_glyph(sf.glyph_id(ch).with_scale_and_position(sf.scale, point(0.0, 0.0))))
            .map(|g| g.px_bounds().min.y)
            .fold(0.0, f32::min)
    }

    fn text_at(&mut self, x: f32, y: f32, s: &str, f: Font_, c: Rgb, anchor: Anchor) {
        let sf = f.scaled();
        let (asc, desc) = (sf.ascent(), sf.descent());
        let width = self.text_len(s, f);
        let x0 = match anchor {
            Anchor::Mt | Anchor::Mm => x - width / 2.0,
            Anchor::Ra => x - width,
            _ => x,
        };
        let baseline = match anchor {
            Anchor::La | Anchor::Ra => y + asc,
            Anchor::Ls => y,
            Anchor::Mt => y - self.ink_top(s, f),
            Anchor::Mm => y + (asc + desc) / 2.0,
        };
        let mut pen = x0;
        let mut prev = None;
        for ch in s.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                pen += sf.kern(p, id);
            }
            if let Some(og) = sf.outline_glyph(id.with_scale_and_position(sf.scale, point(pen, baseline))) {
                let b = og.px_bounds();
                og.draw(|gx, gy, cov| self.blend(b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, c, cov));
            }
            pen += sf.h_advance(id);
            prev = Some(id);
        }
    }

    fn text(&mut self, x: f32, y: f32, s: &str, f: Font_, c: Rgb) {
        self.text_at(x, y, s, f, c, Anchor::La);
    }

    /// Letter-spaced uppercase label. right = anchor at the right end of the run.
    fn caps(&mut self, x: f32, y: f32, text: &str, size: f32, fill: Option<Rgb>, spacing: f32, right: bool) -> f32 {
        let f = num(size, Face::NumSemi);
        let text = text.to_uppercase();
        let width: f32 = text.chars().map(|ch| self.text_len(&ch.to_string(), f)).sum::<f32>()
            + spacing * (text.chars().count().saturating_sub(1)) as f32;
        let mut x = if right { x - width } else { x };
        for ch in text.chars() {
            let s = ch.to_string();
            self.text(x, y, &s, f, fill.unwrap_or(LABEL));
            x += self.text_len(&s, f) + spacing;
        }
        width
    }

    /// Soft colored bloom around bright strokes inside the box (clock digits, graph lines).
    fn glow(&mut self, x0: u32, y0: u32, x1: u32, y1: u32) {
        let (color, strength) = GLOW;
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut mask: Vec<f32> = (0..w * h)
            .map(|i| {
                let c = self.get((x0 as usize + i % w) as f32, (y0 as usize + i / w) as f32);
                let l = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32; // PIL "L"
                if l > 150.0 { 255.0 } else { 0.0 }
            })
            .collect();
        gaussian_blur(&mut mask, w, h, 10.0);
        for i in 0..w * h {
            let m = mask[i] * strength / 255.0;
            if m <= 0.002 {
                continue;
            }
            let (x, y) = (x0 as f32 + (i % w) as f32, y0 as f32 + (i / w) as f32);
            let old = self.get(x, y);
            // screen(old, color * m)
            let n = [0, 1, 2].map(|k| {
                let b = color[k] as f32 * m;
                255.0 - (255.0 - old[k] as f32) * (255.0 - b) / 255.0
            });
            let px = &mut self.pm.pixels_mut()[(y as u32 * W + x as u32) as usize];
            *px = tiny_skia::PremultipliedColorU8::from_rgba(n[0] as u8, n[1] as u8, n[2] as u8, 255).unwrap();
        }
    }

    /// Copy landscape columns `cols` (all rows) from `src`.
    pub fn restore_columns(&mut self, src: &Canvas, cols: std::ops::Range<usize>) {
        let w = W as usize * 4;
        let (a, b) = (cols.start.min(W as usize) * 4, cols.end.min(W as usize) * 4);
        let (dst, src) = (self.pm.data_mut(), src.pm.data());
        for row in 0..H as usize {
            dst[row * w + a..row * w + b].copy_from_slice(&src[row * w + a..row * w + b]);
        }
    }

    pub fn pixmap(&self) -> &Pixmap {
        &self.pm
    }

    pub fn pixmap_mut(&mut self) -> &mut Pixmap {
        &mut self.pm
    }

    pub fn rgb(&self) -> Vec<u8> {
        self.pm.pixels().iter().flat_map(|p| [p.red(), p.green(), p.blue()]).collect()
    }

    pub fn save_png(&self, path: &str) -> anyhow::Result<()> {
        self.pm.save_png(path)?;
        Ok(())
    }
}

/// Gaussian blur approximated by three box blurs (as PIL does).
fn gaussian_blur(data: &mut [f32], w: usize, h: usize, sigma: f32) {
    let n = 3.0;
    let wl = ((12.0 * sigma * sigma / n + 1.0).sqrt()).floor() as i32;
    let wl = if wl % 2 == 0 { wl - 1 } else { wl };
    let m = ((12.0 * sigma * sigma - n * (wl * wl) as f32 - 4.0 * n * wl as f32 - 3.0 * n) / (-4.0 * wl as f32 - 4.0)).round() as i32;
    let mut tmp = vec![0.0; data.len()];
    for pass in 0..3 {
        let r = ((if pass < m { wl } else { wl + 2 }) - 1) / 2;
        box_blur(data, &mut tmp, w, h, r as usize, true);
        box_blur(&tmp, data, w, h, r as usize, false);
    }
}

fn box_blur(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize, horizontal: bool) {
    let (len, lines) = if horizontal { (w, h) } else { (h, w) };
    let idx = |line: usize, i: usize| if horizontal { line * w + i } else { i * w + line };
    let norm = 1.0 / (2 * r + 1) as f32;
    for line in 0..lines {
        let mut acc = 0.0;
        for i in 0..=r.min(len - 1) {
            acc += src[idx(line, i)];
        }
        for i in 0..len {
            dst[idx(line, i)] = acc * norm;
            if i + r + 1 < len {
                acc += src[idx(line, i + r + 1)];
            }
            if i >= r {
                acc -= src[idx(line, i - r)];
            }
        }
    }
}

// --- helpers --------------------------------------------------------------------------

fn temp_color(t: f64) -> Rgb {
    if t < 70.0 { INK } else if t < 85.0 { WARN } else { HOT }
}

fn fmt_left(reset: Option<f64>) -> String {
    let Some(reset) = reset else { return String::new() };
    let s = (reset - epoch_now()).max(0.0) as i64;
    let (d, h, m) = (s / 86400, s % 86400 / 3600, s % 3600 / 60);
    if d > 0 { format!("{d}일 {h}시간") } else if h > 0 { format!("{h}시간 {m}분") } else { format!("{m}분") }
}

fn err_text(e: &AiErr) -> String {
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
fn fmt_rate(bytes_per_s: f64) -> (String, &'static str) {
    let bits = bytes_per_s * 8.0;
    for (unit, scale) in [("Gbps", 1e9), ("Mbps", 1e6), ("Kbps", 1e3)] {
        if bits >= scale {
            let v = bits / scale;
            return (if v < 100.0 { format!("{v:.1}") } else { format!("{v:.0}") }, unit);
        }
    }
    (format!("{bits:.0}"), "bps")
}

fn thousands(v: f64) -> String {
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

// --- weather glyphs (solid, monochrome) --------------------------------------------------

fn weather_icon(cv: &mut Canvas, cx: f32, cy: f32, kind: &str, s: f32) {
    // Cut-outs use the actual ground color here, not BG, so they vanish on the gradient
    let ground = cv.get(cx - 60.0 * s, cy - 40.0 * s);
    let (sun, cloud) = (INK, mix(INK, ground, 0.62));
    let bx = |x0: f32, y0: f32, x1: f32, y1: f32| (cx + x0 * s, cy + y0 * s, cx + x1 * s, cy + y1 * s);
    let pt = |x: f32, y: f32| (cx + x * s, cy + y * s);
    let cloud_shape = |cv: &mut Canvas, ox: f32, oy: f32, fill: Rgb, grow: f32| {
        for e in [(-36.0, -4.0, 4.0, 30.0), (-16.0, -24.0, 28.0, 20.0), (8.0, -8.0, 40.0, 30.0)] {
            let b = bx(ox + e.0 - grow, oy + e.1 - grow, ox + e.2 + grow, oy + e.3 + grow);
            cv.ellipse(b.0, b.1, b.2, b.3, fill);
        }
        let b = bx(ox - 20.0, oy + 10.0 - grow, ox + 24.0, oy + 30.0 + grow);
        cv.rect(b.0, b.1, b.2, b.3, fill);
    };
    let lw = (4.0 * s).round().max(2.0);
    if kind == "sun" || kind == "partly" {
        let r = if kind == "sun" { 16.0 } else { 13.0 };
        let c = if kind == "partly" { (-8.0, -10.0) } else { (0.0, 0.0) };
        let b = bx(c.0 - r, c.1 - r, c.0 + r, c.1 + r);
        cv.ellipse(b.0, b.1, b.2, b.3, sun);
        for i in 0..8 {
            let a = i as f32 * std::f32::consts::PI / 4.0;
            cv.line(&[pt(c.0 + a.cos() * (r + 7.0), c.1 + a.sin() * (r + 7.0)), pt(c.0 + a.cos() * (r + 15.0), c.1 + a.sin() * (r + 15.0))], lw, sun);
        }
    }
    if kind == "moon" || kind == "partly_night" {
        let c = if kind == "partly_night" { (-8.0, -10.0) } else { (0.0, 0.0) };
        let b = bx(c.0 - 24.0, c.1 - 24.0, c.0 + 24.0, c.1 + 24.0);
        cv.ellipse(b.0, b.1, b.2, b.3, sun);
        let b = bx(c.0 - 8.0, c.1 - 32.0, c.0 + 36.0, c.1 + 12.0);
        cv.ellipse(b.0, b.1, b.2, b.3, ground);
    }
    if ["partly", "partly_night", "cloud", "rain", "snow", "storm", "fog"].contains(&kind) {
        let (ox, oy) = if kind.starts_with("partly") { (14.0, 16.0) } else { (0.0, 0.0) };
        if kind.starts_with("partly") {
            cloud_shape(cv, ox, oy, ground, 5.0); // gap between sun/moon and cloud
        }
        cloud_shape(cv, ox, oy, cloud, 0.0);
    }
    match kind {
        "rain" => {
            for i in [-18.0, 0.0, 18.0] {
                cv.line(&[pt(i, 40.0), pt(i - 6.0, 56.0)], lw, INK);
            }
        }
        "snow" => {
            for i in [-18.0, 0.0, 18.0] {
                let b = bx(i - 4.0, 44.0, i + 4.0, 52.0);
                cv.ellipse(b.0, b.1, b.2, b.3, INK);
            }
        }
        "storm" => cv.polygon(&[pt(2.0, 32.0), pt(-10.0, 50.0), pt(0.0, 50.0), pt(-6.0, 64.0), pt(12.0, 44.0), pt(2.0, 44.0)], WARN),
        "fog" => {
            for i in [40.0, 52.0] {
                cv.line(&[pt(-34.0, i), pt(34.0, i)], lw, cloud);
            }
        }
        _ => {}
    }
}

// --- column A: clock, calendar, weather -----------------------------------------------

const CLOCK_BASELINE: f32 = 172.0;
const DATE_Y: f32 = 206.0;

fn draw_clock(cv: &mut Canvas, now: &DateTime<Local>, x: f32, right: f32) {
    let h12 = if now.hour() % 12 == 0 { 12 } else { now.hour() % 12 };
    let hm = format!("{}:{:02}", h12, now.minute());
    let ampm = if now.hour() < 12 { "AM" } else { "PM" };
    let sec_font = num(34.0, Face::NumLight);
    let mut f = num(170.0, Face::NumLight);
    for size in (120..=170).rev().step_by(5) {
        // 1-9 o'clock can be larger than 10-12
        f = num(size as f32, Face::NumLight);
        if x + cv.text_len(&hm, f) + 16.0 + cv.text_len("00", sec_font) <= right {
            break;
        }
    }
    cv.text_at(x - 4.0, CLOCK_BASELINE, &hm, f, INK, Anchor::Ls);
    let digits_top = CLOCK_BASELINE + cv.ink_top("0", f);
    let sx = x + cv.text_len(&hm, f) + 16.0;
    cv.caps(sx, digits_top + 4.0, ampm, 20.0, Some(ACCENT), 2.5, false);
    cv.text(sx, digits_top + 30.0, &format!("{:02}", now.second()), sec_font, mix(ACCENT, BG, 0.7));
    let date = format!("{}. {:02}. {:02}   {}요일", now.year(), now.month(), now.day(), WEEKDAYS[now.weekday().num_days_from_monday() as usize]);
    cv.text(x, DATE_Y, &date, ko(24.0, false), MUTED);
}

fn draw_calendar(cv: &mut Canvas, now: &DateTime<Local>, x: f32, y: f32, w: f32) {
    let cw = w / 7.0;
    cv.caps(x + 4.0, y, &now.format("%B").to_string(), 16.0, Some(MUTED), 2.5, false);
    let (f_head, f_day) = (ko(15.0, false), num(21.0, Face::Num));
    for (i, name) in ["일", "월", "화", "수", "목", "금", "토"].iter().enumerate() {
        let c = if i == 0 { SUNDAY } else if i == 6 { SATURDAY } else { MUTED };
        cv.text_at(x + cw * i as f32 + cw / 2.0, y + 32.0, name, f_head, c, Anchor::Mt);
    }
    // Sunday-first month grid
    let first = now.with_day(1).unwrap();
    let lead = first.weekday().num_days_from_sunday() as usize;
    let days = {
        let (ny, nm) = if now.month() == 12 { (now.year() + 1, 1) } else { (now.year(), now.month() + 1) };
        chrono::NaiveDate::from_ymd_opt(ny, nm, 1).unwrap().pred_opt().unwrap().day() as usize
    };
    for day in 1..=days {
        let cell = lead + day - 1;
        let (r, i) = (cell / 7, cell % 7);
        let cy = y + 70.0 + r as f32 * 27.0;
        let cx = x + cw * i as f32 + cw / 2.0;
        let red = i == 0 || is_holiday(now.year(), now.month(), day as u32);
        let mut color = if red { SUNDAY } else if i == 6 { SATURDAY } else { INK };
        if day as u32 == now.day() {
            cv.rect(cx - 15.0, cy - 13.0, cx + 15.0, cy + 13.0, ACCENT);
            color = BG;
        }
        cv.text_at(cx, cy, &day.to_string(), f_day, color, Anchor::Mm);
    }
}

fn air_color(level: u8) -> Rgb {
    // AirKorea palette, softened for the dark theme
    [[110, 170, 255], [110, 205, 140], [236, 190, 80], [236, 120, 80], HOT][level as usize]
}

/// "초미세 7 좋음   미세 8 좋음": value plus the Korean grade in its color, shrunk to fit `right`.
fn draw_air(cv: &mut Canvas, w: &Weather, x: f32, y: f32, right: f32) {
    let items: Vec<(&str, Option<String>, Option<(&str, u8)>)> = [("초미세", w.pm25, true), ("미세", w.pm10, false)]
        .into_iter()
        .map(|(label, pm, fine)| (label, pm.map(|v| format!("{v:.0}")), pm.map(|v| air_grade(v, fine))))
        .collect();
    for size in [18.0, 17.0, 16.0, 15.0, 14.0] {
        let (f_lab, f_val, f_grade) = (ko(size, false), num(size + 4.0, Face::Num), ko(size, true));
        let (gap, sep) = (size * 0.3, size * 1.0);
        let width: f32 = items
            .iter()
            .map(|(label, v, g)| {
                cv.text_len(label, f_lab) + gap + cv.text_len(v.as_deref().unwrap_or("--"), f_val)
                    + g.map_or(0.0, |(t, _)| gap + cv.text_len(t, f_grade))
            })
            .sum::<f32>()
            + sep;
        if x + width > right && size > 14.0 {
            continue;
        }
        let mut cx = x;
        for (label, v, g) in &items {
            cv.text(cx, y, label, f_lab, MUTED);
            cx += cv.text_len(label, f_lab) + gap;
            let v = v.as_deref().unwrap_or("--");
            cv.text(cx, y - 1.0, v, f_val, if g.is_some() { INK } else { FAINT });
            cx += cv.text_len(v, f_val);
            if let Some((grade, level)) = g {
                cx += gap;
                cv.text(cx, y, grade, f_grade, air_color(*level));
                cx += cv.text_len(grade, f_grade);
            }
            cx += sep;
        }
        break;
    }
}

/// Sunrise / sunset in 12-hour time with small up/down glyphs, right-aligned to `right`.
fn draw_sun(cv: &mut Canvas, w: &Weather, min_x: f32, y: f32, right: f32) {
    let (Some(rise), Some(set)) = (w.sunrise, w.sunset) else { return };
    let h12 = |t: chrono::NaiveTime| format!("{}:{:02}", if t.hour() % 12 == 0 { 12 } else { t.hour() % 12 }, t.minute());
    let (a, b) = (h12(rise), h12(set));
    let glyph = 18.0;
    // shrink before giving up when the min/max row is long ("강수 100%")
    let fit = [(22.0, 22.0), (20.0, 16.0), (18.0, 12.0)].into_iter().map(|(z, gap)| {
        let f = num(z, Face::Num);
        (f, gap, glyph + 6.0 + cv.text_len(&a, f) + gap + glyph + 6.0 + cv.text_len(&b, f))
    }).find(|&(_, _, w)| right - w >= min_x);
    let Some((f, gap, width)) = fit else { return };
    let x = right - width;
    let mut cx = x;
    for (j, t) in [a, b].iter().enumerate() {
        // half sun on the horizon with an arrow: up = sunrise, down = sunset
        let (gx, gy) = (cx, y + 20.0);
        cv.line(&[(gx, gy), (gx + glyph, gy)], 1.0, MUTED);
        let r = 6.0;
        cv.polygon(&(0..=12).map(|k| {
            let a = std::f32::consts::PI * k as f32 / 12.0;
            (gx + glyph / 2.0 + r * a.cos(), gy - 2.0 - r * a.sin())
        }).collect::<Vec<_>>(), if j == 0 { WARN } else { mix(WARN, BG, 0.6) });
        let (ax, ay) = (gx + glyph / 2.0, gy - 11.0);
        if j == 0 {
            cv.polygon(&[(ax - 4.0, ay), (ax + 4.0, ay), (ax, ay - 5.0)], MUTED);
        } else {
            cv.polygon(&[(ax - 4.0, ay - 5.0), (ax + 4.0, ay - 5.0), (ax, ay)], MUTED);
        }
        cx += glyph + 6.0;
        cv.text(cx, y + 1.0, t, f, SOFT);
        cx += cv.text_len(t, f) + gap;
    }
}

const TOMORROW_W: f32 = 122.0; // tomorrow block, from its left edge to the column's right edge
const TOMORROW_ICON_R: f32 = 20.0; // half the width of the small weather icon

fn draw_weather(cv: &mut Canvas, st: &State, x: f32, y: f32, right: f32) {
    let Some(w) = &st.weather else {
        cv.text(x, y + 20.0, "날씨 불러오는 중…", ko(22.0, false), MUTED);
        return;
    };
    weather_icon(cv, x + 44.0, y + 58.0, w.icon, 0.95);
    let big = num(78.0, Face::NumLight);
    let t = format!("{:.0}°", w.temp);
    cv.text(x + 108.0, y + 14.0, &t, big, INK);
    let tx = x + 108.0 + cv.text_len(&t, big) + 22.0;
    cv.text(tx, y + 26.0, w.label, ko(26.0, true), INK);
    cv.text(tx, y + 64.0, &format!("체감 {:.0}°  ·  습도 {}%", w.feels, w.humidity), ko(20.0, false), MUTED);
    draw_air(cv, w, tx, y + 92.0, right - TOMORROW_W - 36.0);
    let rain = w.rain_prob.map(|p| format!("  ·  강수 {p}%")).unwrap_or_default();
    let minmax = format!("최저 {:.0}°  ·  최고 {:.0}°{rain}", w.t_min, w.t_max);
    cv.text(x, y + 124.0, &minmax, ko(20.0, false), MUTED);
    draw_sun(cv, w, x + cv.text_len(&minmax, ko(20.0, false)) + 16.0, y + 124.0, right - TOMORROW_W - 40.0);

    if let Some(t) = &w.tomorrow {
        let tx = right - TOMORROW_W;
        cv.line(&[(tx - 24.0, y + 6.0), (tx - 24.0, y + 150.0)], 1.0, RULE);
        cv.caps(tx, y + 4.0, "Tomorrow", 14.0, Some(MUTED), 2.0, false);
        // icon and label sit a little left of the temperatures below
        weather_icon(cv, tx + TOMORROW_ICON_R - 5.0, y + 58.0, t.icon, 0.5);
        cv.text(tx + TOMORROW_ICON_R * 2.0 + 5.0, y + 44.0, t.label, ko(18.0, false), INK);
        cv.text(tx, y + 96.0, &format!("{:.0}° / {:.0}°", t.t_min, t.t_max), num(26.0, Face::Num), INK);
        if let Some(p) = t.rain_prob {
            cv.text(tx, y + 128.0, &format!("강수 {p}%"), ko(17.0, false), MUTED);
        }
    }
}

// --- column B: hardware ------------------------------------------------------------------

/// Scrolling area chart, newest at the right edge, 0-100 %.
fn graph(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, samples: &VecDeque<Option<f64>>, hot: bool) {
    for frac in [0.0, 0.5] {
        let gy = y + (h * frac).floor();
        let mut gx = x;
        while gx < x + w {
            cv.point(gx, gy, RULE); // dotted guide
            gx += 6.0;
        }
    }
    cv.line(&[(x, y + h), (x + w, y + h)], 1.0, RULE);
    let mut pts = Vec::new();
    for (i, v) in samples.iter().rev().enumerate() {
        let px = x + w - i as f32 * GRAPH_STEP;
        let Some(v) = v else { break };
        if px < x {
            break;
        }
        pts.push((px, y + h - h * (v.clamp(0.0, 100.0) as f32) / 100.0));
    }
    if pts.len() >= 2 {
        let line = if hot { HOT } else { ACCENT };
        let mut poly = pts.clone();
        poly.push((pts[pts.len() - 1].0, y + h));
        poly.push((pts[0].0, y + h));
        cv.polygon_alpha(&poly, line, 0.16);
        cv.line(&pts, 2.0, line);
    }
}

/// Faint auto-scaled area chart behind a value: 0..max (at least `floor`), or with `span`,
/// the recent min..max at least `span` wide (temperatures).
fn bg_graph(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, samples: &VecDeque<Option<f64>>, floor: f64, span: Option<f64>) {
    let vals: Vec<f64> = samples.iter().flatten().copied().collect();
    if vals.len() < 2 {
        return;
    }
    let (min, max) = vals.iter().fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let (lo, hi) = match span {
        None => (0.0, (max * 1.15).max(floor)),
        Some(span) => {
            let pad = (span - (max - min)).max(0.0) / 2.0 + (max - min) * 0.15;
            (min - pad, max + pad)
        }
    };
    let step = w / (HIST_LEN - 1) as f32;
    let mut pts = Vec::new();
    for (i, v) in samples.iter().rev().enumerate() {
        let px = x + w - i as f32 * step;
        let Some(v) = v else { break };
        if px < x {
            break;
        }
        pts.push((px, y + h - h * (((v - lo) / (hi - lo)).clamp(0.0, 1.0) as f32)));
    }
    if pts.len() >= 2 {
        let mut poly = pts.clone();
        poly.push((pts[pts.len() - 1].0, y + h));
        poly.push((pts[0].0, y + h));
        cv.polygon(&poly, BG_FILL);
        cv.line(&pts, 1.0, BG_LINE);
    }
}

/// Faint per-core usage grid (two rows) behind the CPU graph, 0-100 %, 1 px per second.
fn core_graphs(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, cores: &[VecDeque<Option<f64>>]) {
    if cores.is_empty() {
        return;
    }
    let gap = 4.0;
    let cols = cores.len().div_ceil(2);
    let cw = ((w - gap * (cols - 1) as f32) / cols as f32).floor();
    let ch = ((h - gap) / 2.0).floor();
    for (i, samples) in cores.iter().enumerate() {
        let cx = x + (i % cols) as f32 * (cw + gap);
        let cy = y + (i / cols) as f32 * (ch + gap);
        let mut pts = Vec::new();
        for (j, v) in samples.iter().rev().enumerate() {
            let px = cx + cw - j as f32;
            let Some(v) = v else { break };
            if px < cx {
                break;
            }
            pts.push((px, cy + ch - ch * (v.clamp(0.0, 100.0) as f32) / 100.0));
        }
        if pts.len() >= 2 {
            let mut poly = pts.clone();
            poly.push((pts[pts.len() - 1].0, cy + ch));
            poly.push((pts[0].0, cy + ch));
            cv.polygon(&poly, BG_FILL);
            cv.line(&pts, 1.0, BG_LINE);
        }
        // faint cell frame, drawn last so the graph never hides it
        cv.line(&[(cx, cy), (cx + cw, cy), (cx + cw, cy + ch), (cx, cy + ch), (cx, cy)], 1.0, CORE_BOX);
    }
}

fn draw_hardware(cv: &mut Canvas, st: &State, x: f32, right: f32) {
    let hw = &st.hw;
    let rows = [
        ("CPU", "cpu", hw.cpu.map(|v| format!("{v:.0}")), None, "%", hw.cpu_temp, Some(hw.cpu_power)),
        ("Memory", "ram", hw.ram_used.map(|v| format!("{v:.1}")), hw.ram_total, "GB", hw.ram_temp, None),
        ("GPU", "gpu", hw.gpu.map(|v| format!("{v:.0}")), None, "%", hw.gpu_temp, Some(hw.gpu_power)),
        ("VRAM", "vram", hw.vram_used.map(|v| format!("{v:.1}")), hw.vram_total, "GB", hw.vram_temp, None),
    ];
    let gx = x + 150.0;
    for (i, (label, key, val, total, unit, temp, power)) in rows.into_iter().enumerate() {
        let y = TOP + i as f32 * 84.0;
        cv.caps(x, y, label, 16.0, None, 2.5, false);
        if let Some(power) = power {
            // package / board power, right-aligned in the gap before the graph
            let (fv, fu) = (num(24.0, Face::Num), num(15.0, Face::NumSemi));
            let px = gx - 22.0 - cv.text_len("W", fu);
            cv.text(px + 3.0, y + 40.0, "W", fu, MUTED);
            let v = power.map_or("--".to_string(), |w| format!("{w:.0}"));
            cv.text(px - cv.text_len(&v, fv), y + 31.0, &v, fv, if power.is_some() { SOFT } else { FAINT });
        }
        match val {
            None => cv.text(x, y + 24.0, "--", num(38.0, Face::Num), MUTED),
            Some(val) => {
                let f = num(40.0, Face::Num);
                cv.text(x, y + 22.0, &val, f, INK);
                let small = num(18.0, Face::NumSemi);
                let mut ux = x + cv.text_len(&val, f) + 5.0;
                if let Some(total) = total {
                    // used/capacity: capacity as small as the unit
                    let t = format!("/{total:.0}");
                    cv.text(ux - 2.0, y + 38.0, &t, small, SOFT);
                    ux += cv.text_len(&t, small) + 1.0;
                }
                cv.text(ux, y + 38.0, unit, small, MUTED);
            }
        }
        let hist = &st.hist[key];
        let hot = hist.back().copied().flatten().is_some_and(|v| v >= 90.0);
        if key == "cpu" {
            core_graphs(cv, gx, y + 8.0, GRAPH_W, GRAPH_H, &st.cores);
        }
        graph(cv, gx, y + 8.0, GRAPH_W, GRAPH_H, hist, hot);
        // temperature just after the usage graph, over its own faint history
        let tx = gx + GRAPH_W + 26.0;
        let temp_key = match key { "cpu" => "cpu_temp", "ram" => "ram_temp", "gpu" => "gpu_temp", _ => "vram_temp" };
        bg_graph(cv, tx - 4.0, y + 8.0, right - tx + 4.0, GRAPH_H, &st.hist[temp_key], 0.0, Some(10.0));
        let tf = num(36.0, Face::NumLight);
        match temp {
            None => cv.text(tx, y + 24.0, "--°", tf, FAINT),
            Some(t) => cv.text(tx, y + 24.0, &format!("{t:.0}°"), tf, temp_color(t)),
        }
        if i == 0 {
            cv.caps(tx, y, "Temp", 14.0, Some(MUTED), 2.0, false);
        }
    }

    let y = TOP + 4.0 * 84.0 + 6.0;
    cv.line(&[(x, y - 10.0), (right, y - 10.0)], 1.0, RULE);
    cv.caps(x, y + 4.0, "Network", 16.0, None, 2.5, false);
    for (j, (label, key, rate)) in [("Down", "net_down", hw.net_down), ("Up", "net_up", hw.net_up)].into_iter().enumerate() {
        let nx = gx + j as f32 * 200.0;
        bg_graph(cv, nx, y + 2.0, 180.0, 66.0, &st.hist[key], 125_000.0, None); // floor 1 Mbps
        cv.caps(nx, y + 4.0, label, 14.0, Some(MUTED), 2.0, false);
        let f = num(32.0, Face::Num);
        match rate {
            None => cv.text(nx, y + 26.0, "--", f, FAINT),
            Some(rate) => {
                let (val, unit) = fmt_rate(rate);
                cv.text(nx, y + 26.0, &val, f, INK);
                cv.text(nx + cv.text_len(&val, f) + 5.0, y + 38.0, unit, num(16.0, Face::NumSemi), MUTED);
            }
        }
    }
}

// --- column C: fans, AI usage ----------------------------------------------------------------

const AI_C1: f32 = 196.0;
const AI_C2: f32 = 382.0;

fn draw_fans(cv: &mut Canvas, st: &State, x: f32, right: f32) {
    cv.caps(x, TOP, "Fans", 16.0, None, 2.5, false);
    cv.caps(right, TOP, "RPM", 14.0, Some(MUTED), 2.0, true);
    let hw = &st.hw;
    let fans = [("라디에이터", "fan_radiator", hw.fan_radiator), ("펌프", "fan_pump", hw.fan_pump), ("VGA", "fan_gpu", hw.fan_gpu)];
    for (j, (label, key, rpm)) in fans.into_iter().enumerate() {
        let fx = x + [0.0, AI_C1, AI_C2][j];
        bg_graph(cv, fx, TOP + 24.0, 170.0, 64.0, &st.hist[key], 500.0, None);
        cv.text(fx, TOP + 26.0, label, ko(16.0, false), MUTED);
        let f = num(32.0, Face::Num);
        match rpm {
            Some(r) => cv.text(fx, TOP + 48.0, &thousands(r), f, INK),
            None => cv.text(fx, TOP + 48.0, "--", f, FAINT),
        }
    }
}

/// Blocky bar (Minecraft style): each block is 100/segments %, the last one fills partially.
fn segmented_bar(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, pct: f64, color: Rgb) {
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

const AI_TOP: f32 = 150.0;
const AI_ROW: f32 = 90.0;

/// Strips under the AI names where the mascots roam (Claude, Codex, Antigravity).
pub fn mascot_areas() -> [crate::mascot::Area; 3] {
    let x = COL_C.0;
    [0, 1, 2].map(|i| crate::mascot::Area { x0: x + 30.0, x1: x + AI_C1 - 30.0, ground: AI_TOP + 34.0 + i as f32 * AI_ROW + 86.0 })
}

fn draw_ai(cv: &mut Canvas, st: &State, x: f32, top: f32) {
    let (c1, c2) = (x + AI_C1, x + AI_C2);
    let bw = 150.0;
    let f_val = num(32.0, Face::Num);
    cv.caps(x, top, "AI Left", 16.0, None, 2.5, false);
    cv.caps(c1, top, "5 Hours", 14.0, Some(MUTED), 2.0, false);
    cv.caps(c2, top, "Weekly", 14.0, Some(MUTED), 2.0, false);
    for (i, name) in AI_NAMES.iter().enumerate() {
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
            let when = if refilled { "초기화됨 · 확인 중".to_string() } else { fmt_left(reset) };
            cv.text(cx, y + 58.0, &when, ko(16.0, false), SOFT);
        }
    }
}

// --- standby -------------------------------------------------------------------------------

/// Standby screens built by ../scripts/standby_pages.py (art generated with Codex).
static STANDBY: [&[u8]; 3] = [
    include_bytes!("../../assets/standby/page_1.png"),
    include_bytes!("../../assets/standby/page_2.png"),
    include_bytes!("../../assets/standby/page_3.png"),
];

/// One of the standby screens at random (shown at shutdown; stays on the panel until the next start).
pub fn standby() -> Canvas {
    let pick = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) as usize % STANDBY.len();
    standby_page(pick)
}

pub fn standby_page(i: usize) -> Canvas {
    match Pixmap::decode_png(STANDBY[i]) {
        Ok(pm) => Canvas { pm },
        Err(e) => {
            log::warn!("standby page {i}: {e}");
            Canvas::background()
        }
    }
}

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
