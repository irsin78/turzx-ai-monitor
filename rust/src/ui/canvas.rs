//! Drawing surface: fonts, text, shapes and the glow effect on a 1920x480 pixmap.

use super::*;

#[derive(Clone, Copy)]
pub enum Face {
    NumLight,
    Num,
    NumSemi,
    Ko,
    KoBold,
}

pub(super) struct Fonts {
    num_light: FontRef<'static>,
    num: FontRef<'static>,
    num_semi: FontRef<'static>,
    noto: FontRef<'static>,
    noto_bold: FontRef<'static>,
    /// the language's preferred Windows font (Japanese / Chinese glyph forms), regular and bold
    system: Option<(FontRef<'static>, FontRef<'static>)>,
    /// last resort for anything the others lack
    segoe: Option<FontRef<'static>>,
}

pub(super) static HARMONY_THIN: &[u8] = include_bytes!("../../../fonts/HarmonyOS_Sans_Thin.ttf");
pub(super) static HARMONY_LIGHT: &[u8] = include_bytes!("../../../fonts/HarmonyOS_Sans_Light.ttf");
pub(super) static HARMONY_MEDIUM: &[u8] = include_bytes!("../../../fonts/HarmonyOS_Sans_Medium.ttf");
pub(super) static NOTO_KR: &[u8] = include_bytes!("../../../fonts/NotoSansKR-VF.ttf");
pub(super) const NUM_SCALE: f32 = 0.92; // sizes are tuned for Bahnschrift; evens out HarmonyOS metrics

/// A font from C:\Windows\Fonts (first face of a .ttc), kept for the life of the process.
fn windows_font(file: &str) -> Option<FontRef<'static>> {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let data: &'static [u8] = Box::leak(std::fs::read(format!(r"{dir}\Fonts\{file}")).ok()?.into_boxed_slice());
    FontRef::try_from_slice_and_index(data, 0).ok()
}

pub(super) fn fonts() -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| {
        let noto = |wght: f32| {
            let mut f = FontRef::try_from_slice(NOTO_KR).expect("Noto Sans KR");
            f.set_variation(b"wght", wght);
            f
        };
        let system = match crate::i18n::get().font {
            TextFont::Noto => None,
            TextFont::YuGothic => windows_font("YuGothR.ttc").zip(windows_font("YuGothM.ttc")),
            TextFont::YaHei => windows_font("msyh.ttc").zip(windows_font("msyhbd.ttc")),
            TextFont::JhengHei => windows_font("msjh.ttc").zip(windows_font("msjhbd.ttc")),
        };
        Fonts {
            num_light: FontRef::try_from_slice(HARMONY_THIN).expect("HarmonyOS Thin"),
            num: FontRef::try_from_slice(HARMONY_LIGHT).expect("HarmonyOS Light"),
            num_semi: FontRef::try_from_slice(HARMONY_MEDIUM).expect("HarmonyOS Medium"),
            noto: noto(300.0),      // Light
            noto_bold: noto(500.0), // Medium
            system,
            segoe: windows_font("segoeui.ttf"),
        }
    })
}

/// A font face at a PIL-style size (pixels per em).
#[derive(Clone, Copy)]
pub struct Font_ {
    face: Face,
    size: f32,
}

pub(super) fn num(size: f32, face: Face) -> Font_ {
    Font_ { face, size: (size * NUM_SCALE).round() }
}

/// Text in the screen language (`ko` for historical reasons: it was Hangul only).
pub(super) fn ko(size: f32, bold: bool) -> Font_ {
    Font_ { face: if bold { Face::KoBold } else { Face::Ko }, size }
}

type Scaled = ab_glyph::PxScaleFont<&'static FontRef<'static>>;

/// Fonts to try for each character, preferred first, per face (built once).
fn chains() -> &'static [Vec<&'static FontRef<'static>>; 5] {
    static C: OnceLock<[Vec<&'static FontRef<'static>>; 5]> = OnceLock::new();
    C.get_or_init(|| {
        let f = fonts();
        let build = |face: Face| {
            let bold = matches!(face, Face::KoBold | Face::NumSemi);
            let mut v: Vec<&'static FontRef<'static>> = Vec::with_capacity(4);
            match face {
                Face::NumLight => v.push(&f.num_light),
                Face::Num => v.push(&f.num),
                Face::NumSemi => v.push(&f.num_semi),
                Face::Ko | Face::KoBold => {}
            }
            v.extend(f.system.as_ref().map(|s| if bold { &s.1 } else { &s.0 }));
            v.push(if bold { &f.noto_bold } else { &f.noto });
            if matches!(face, Face::Ko | Face::KoBold) {
                v.push(if bold { &f.num_semi } else { &f.num }); // Latin Extended (Polish, Turkish)
            }
            v.extend(f.segoe.as_ref());
            v
        };
        [Face::NumLight, Face::Num, Face::NumSemi, Face::Ko, Face::KoBold].map(build)
    })
}

impl Font_ {
    fn scale(&self, f: &'static FontRef<'static>) -> Scaled {
        let em = f.units_per_em().unwrap_or(1000.0);
        f.as_scaled(PxScale::from(self.size * f.height_unscaled() / em))
    }

    /// The primary font at this size (line metrics come from it).
    pub(super) fn scaled(&self) -> Scaled {
        self.scale(chains()[self.face as usize][0])
    }

    /// Call `f` with each character and the font that has it (no allocation).
    pub(super) fn each(&self, s: &str, mut f: impl FnMut(char, Scaled)) {
        let chain = &chains()[self.face as usize];
        let mut scaled: [Option<Scaled>; 6] = [None; 6];
        for ch in s.chars() {
            let mut pick = None;
            for (i, font) in chain.iter().enumerate().take(scaled.len()) {
                if font.glyph_id(ch).0 != 0 {
                    pick = Some(*scaled[i].get_or_insert_with(|| self.scale(font)));
                    break;
                }
            }
            f(ch, pick.unwrap_or_else(|| *scaled[0].get_or_insert_with(|| self.scale(chain[0]))));
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Anchor {
    La, // left, ascender (PIL default)
    Ls, // left, baseline
    Mt, // middle, top of ink
    Mm, // middle, middle of ascender/descender
}

// --- canvas -------------------------------------------------------------------------

#[derive(Clone)]
pub struct Canvas {
    pub(super) pm: Pixmap,
}

pub(super) fn paint(c: Rgb) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], 255);
    p.anti_alias = true;
    p
}

impl Canvas {
    pub(super) fn background() -> Canvas {
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

    pub(super) fn get(&self, x: f32, y: f32) -> Rgb {
        let (x, y) = (x.clamp(0.0, W as f32 - 1.0) as u32, y.clamp(0.0, H as f32 - 1.0) as u32);
        let p = self.pm.pixels()[(y * W + x) as usize];
        [p.red(), p.green(), p.blue()]
    }

    pub(super) fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
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
    pub(super) fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgb) {
        if let Some(r) = Rect::from_ltrb(x0, y0, x1 + 1.0, y1 + 1.0) {
            let mut p = paint(c);
            p.anti_alias = false;
            self.pm.fill_rect(r, &p, Transform::identity(), None);
        }
    }

    pub(super) fn ellipse(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgb) {
        if let Some(r) = Rect::from_ltrb(x0, y0, x1 + 1.0, y1 + 1.0) {
            if let Some(path) = PathBuilder::from_oval(r) {
                self.pm.fill_path(&path, &paint(c), FillRule::Winding, Transform::identity(), None);
            }
        }
    }

    pub(super) fn polygon(&mut self, pts: &[(f32, f32)], c: Rgb) {
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

    pub(super) fn polygon_alpha(&mut self, pts: &[(f32, f32)], c: Rgb, a: f32) {
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
    pub(super) fn line(&mut self, pts: &[(f32, f32)], width: f32, c: Rgb) {
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

    pub(super) fn point(&mut self, x: f32, y: f32, c: Rgb) {
        self.blend(x as i32, y as i32, c, 1.0);
    }

    pub(super) fn text_len(&self, s: &str, f: Font_) -> f32 {
        let (mut w, mut prev) = (0.0, None::<(ab_glyph::GlyphId, *const FontRef)>);
        f.each(s, |ch, sf| {
            let id = sf.glyph_id(ch);
            if let Some((p, font)) = prev {
                if std::ptr::eq(font, sf.font) {
                    w += sf.kern(p, id);
                }
            }
            w += sf.h_advance(id);
            prev = Some((id, sf.font as *const _));
        });
        w
    }

    /// Ink top of `s` relative to its baseline (negative = above).
    pub(super) fn ink_top(&self, s: &str, f: Font_) -> f32 {
        let mut top = 0.0f32;
        f.each(s, |ch, sf| {
            if let Some(g) = sf.outline_glyph(sf.glyph_id(ch).with_scale_and_position(sf.scale, point(0.0, 0.0))) {
                top = top.min(g.px_bounds().min.y);
            }
        });
        top
    }

    pub(super) fn text_at(&mut self, x: f32, y: f32, s: &str, f: Font_, c: Rgb, anchor: Anchor) {
        let sf0 = f.scaled();
        let (asc, desc) = (sf0.ascent(), sf0.descent());
        let x0 = match anchor {
            Anchor::Mt | Anchor::Mm => x - self.text_len(s, f) / 2.0,
            _ => x, // left-aligned: no need to measure
        };
        let baseline = match anchor {
            Anchor::La => y + asc,
            Anchor::Ls => y,
            Anchor::Mt => y - self.ink_top(s, f),
            Anchor::Mm => y + (asc + desc) / 2.0,
        };
        self.glyphs(x0, baseline, s, f, c, 0.0);
    }

    /// Draw `s` from `x` on `baseline`, adding `spacing` between characters; returns the width.
    fn glyphs(&mut self, x: f32, baseline: f32, s: &str, f: Font_, c: Rgb, spacing: f32) -> f32 {
        let (mut pen, mut prev) = (x, None::<(ab_glyph::GlyphId, *const FontRef)>);
        f.each(s, |ch, sf| {
            let id = sf.glyph_id(ch);
            if let Some((p, font)) = prev {
                pen += spacing;
                if spacing == 0.0 && std::ptr::eq(font, sf.font) {
                    pen += sf.kern(p, id);
                }
            }
            if let Some(og) = sf.outline_glyph(id.with_scale_and_position(sf.scale, point(pen, baseline))) {
                let b = og.px_bounds();
                og.draw(|gx, gy, cov| self.blend(b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, c, cov));
            }
            pen += sf.h_advance(id);
            prev = Some((id, sf.font as *const _));
        });
        pen - x
    }

    pub(super) fn text(&mut self, x: f32, y: f32, s: &str, f: Font_, c: Rgb) {
        self.text_at(x, y, s, f, c, Anchor::La);
    }

    /// Letter-spaced uppercase label. right = anchor at the right end of the run.
    pub(super) fn caps(&mut self, x: f32, y: f32, text: &str, size: f32, fill: Option<Rgb>, spacing: f32, right: bool) -> f32 {
        let f = num(size, Face::NumSemi);
        let text = text.to_uppercase();
        // letter-spaced: each character on its own advance, no kerning
        let mut width = 0.0;
        let mut n = 0usize;
        f.each(&text, |ch, sf| {
            width += sf.h_advance(sf.glyph_id(ch));
            n += 1;
        });
        width += spacing * n.saturating_sub(1) as f32;
        let x0 = if right { x - width } else { x };
        self.glyphs(x0, y + f.scaled().ascent(), &text, f, fill.unwrap_or(LABEL), spacing);
        width
    }

    /// Soft colored bloom around bright strokes inside the box (clock digits, graph lines).
    /// Soft colored bloom around bright strokes inside the box (clock digits, graph lines).
    ///
    /// The bright-pixel mask is blurred at half resolution (the glow is wide and soft, so this
    /// looks the same at a quarter of the work) and the blurred mask is reused while the mask
    /// stays the same (the clock changes once a minute).
    pub(super) fn glow(&mut self, x0: u32, y0: u32, x1: u32, y1: u32) {
        let (color, strength) = GLOW;
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
        let stride = W as usize * 4;
        // half-resolution mask: share of bright (PIL "L" > 150) pixels in each 2x2 block
        let mut mask = vec![0f32; hw * hh];
        {
            let px = self.pm.data();
            for y in 0..h {
                let row = &px[(y0 as usize + y) * stride + x0 as usize * 4..][..w * 4];
                let mrow = &mut mask[(y / 2) * hw..][..hw];
                for (x, c) in row.chunks_exact(4).enumerate() {
                    let l = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
                    if l > 150.0 {
                        mrow[x / 2] += 255.0 / 4.0;
                    }
                }
            }
        }
        let key = mask.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &v| (h ^ v.to_bits() as u64).wrapping_mul(0x100_0000_01b3));
        static CACHE: std::sync::Mutex<Vec<((u32, u32), u64, Vec<f32>)>> = std::sync::Mutex::new(Vec::new());
        let mut cache = CACHE.lock().unwrap();
        let slot = match cache.iter().position(|(at, _, _)| *at == (x0, y0)) {
            Some(i) => i,
            None => {
                cache.push(((x0, y0), !key, Vec::new()));
                cache.len() - 1
            }
        };
        if cache[slot].1 != key {
            gaussian_blur(&mut mask, hw, hh, 5.0);
            cache[slot] = ((x0, y0), key, mask);
        }
        let blurred = &cache[slot].2;
        // screen(old, color * m), sampling the half-resolution mask bilinearly
        let px = self.pm.data_mut();
        for y in 0..h {
            let fy = ((y as f32 - 0.5) / 2.0).clamp(0.0, (hh - 1) as f32);
            let (my0, ty) = (fy as usize, fy.fract());
            let my1 = (my0 + 1).min(hh - 1);
            let row = &mut px[(y0 as usize + y) * stride + x0 as usize * 4..][..w * 4];
            for (x, p) in row.chunks_exact_mut(4).enumerate() {
                let fx = ((x as f32 - 0.5) / 2.0).clamp(0.0, (hw - 1) as f32);
                let (mx0, tx) = (fx as usize, fx.fract());
                let mx1 = (mx0 + 1).min(hw - 1);
                let top = blurred[my0 * hw + mx0] * (1.0 - tx) + blurred[my0 * hw + mx1] * tx;
                let bot = blurred[my1 * hw + mx0] * (1.0 - tx) + blurred[my1 * hw + mx1] * tx;
                let m = (top * (1.0 - ty) + bot * ty) * strength / 255.0;
                if m <= 0.002 {
                    continue;
                }
                for k in 0..3 {
                    let b = color[k] as f32 * m;
                    p[k] = (255.0 - (255.0 - p[k] as f32) * (255.0 - b) / 255.0) as u8;
                }
            }
        }
    }

    /// Keep a copy of landscape columns `cols` (all rows) in `out`.
    pub fn save_columns(&self, cols: std::ops::Range<usize>, out: &mut Vec<u8>) {
        let w = W as usize * 4;
        let (a, b) = (cols.start.min(W as usize) * 4, cols.end.min(W as usize) * 4);
        out.clear();
        for row in self.pm.data().chunks_exact(w) {
            out.extend_from_slice(&row[a..b]);
        }
    }

    /// Put back columns saved with `save_columns`.
    pub fn load_columns(&mut self, cols: std::ops::Range<usize>, saved: &[u8]) {
        let w = W as usize * 4;
        let (a, b) = (cols.start.min(W as usize) * 4, cols.end.min(W as usize) * 4);
        for (row, src) in self.pm.data_mut().chunks_exact_mut(w).zip(saved.chunks_exact(b - a)) {
            row[a..b].copy_from_slice(src);
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
pub(super) fn gaussian_blur(data: &mut [f32], w: usize, h: usize, sigma: f32) {
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

pub(super) fn box_blur(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize, horizontal: bool) {
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
