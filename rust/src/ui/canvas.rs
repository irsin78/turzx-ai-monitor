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

impl Font_ {
    /// Fonts to try for each character, preferred first.
    fn chain(&self) -> Vec<&'static FontRef<'static>> {
        let f = fonts();
        let bold = matches!(self.face, Face::KoBold | Face::NumSemi);
        let (system, noto) = (f.system.as_ref().map(|s| if bold { &s.1 } else { &s.0 }), if bold { &f.noto_bold } else { &f.noto });
        let mut v = Vec::with_capacity(4);
        match self.face {
            Face::NumLight => v.push(&f.num_light),
            Face::Num => v.push(&f.num),
            Face::NumSemi => v.push(&f.num_semi),
            Face::Ko | Face::KoBold => {}
        }
        v.extend(system);
        v.push(noto);
        if matches!(self.face, Face::Ko | Face::KoBold) {
            v.push(if bold { &f.num_semi } else { &f.num }); // Latin Extended (Polish, Turkish)
        }
        v.extend(f.segoe.as_ref());
        v
    }

    fn scale(&self, f: &'static FontRef<'static>) -> Scaled {
        let em = f.units_per_em().unwrap_or(1000.0);
        f.as_scaled(PxScale::from(self.size * f.height_unscaled() / em))
    }

    /// The primary font at this size (line metrics come from it).
    pub(super) fn scaled(&self) -> Scaled {
        self.scale(self.chain()[0])
    }

    /// Each character with the font that has it.
    pub(super) fn runs(&self, s: &str) -> Vec<(char, Scaled)> {
        let chain: Vec<Scaled> = self.chain().into_iter().map(|f| self.scale(f)).collect();
        s.chars()
            .map(|ch| {
                let sf = chain.iter().find(|sf| sf.glyph_id(ch).0 != 0).unwrap_or(&chain[0]);
                (ch, *sf)
            })
            .collect()
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
        let mut w = 0.0;
        let mut prev: Option<(ab_glyph::GlyphId, *const FontRef)> = None;
        for (ch, sf) in f.runs(s) {
            let id = sf.glyph_id(ch);
            if let Some((p, font)) = prev {
                if std::ptr::eq(font, sf.font) {
                    w += sf.kern(p, id);
                }
            }
            w += sf.h_advance(id);
            prev = Some((id, sf.font as *const _));
        }
        w
    }

    /// Ink top of `s` relative to its baseline (negative = above).
    pub(super) fn ink_top(&self, s: &str, f: Font_) -> f32 {
        f.runs(s)
            .into_iter()
            .filter_map(|(ch, sf)| sf.outline_glyph(sf.glyph_id(ch).with_scale_and_position(sf.scale, point(0.0, 0.0))))
            .map(|g| g.px_bounds().min.y)
            .fold(0.0, f32::min)
    }

    pub(super) fn text_at(&mut self, x: f32, y: f32, s: &str, f: Font_, c: Rgb, anchor: Anchor) {
        let sf0 = f.scaled();
        let (asc, desc) = (sf0.ascent(), sf0.descent());
        let width = self.text_len(s, f);
        let x0 = match anchor {
            Anchor::Mt | Anchor::Mm => x - width / 2.0,
            _ => x,
        };
        let baseline = match anchor {
            Anchor::La => y + asc,
            Anchor::Ls => y,
            Anchor::Mt => y - self.ink_top(s, f),
            Anchor::Mm => y + (asc + desc) / 2.0,
        };
        let mut pen = x0;
        let mut prev: Option<(ab_glyph::GlyphId, *const FontRef)> = None;
        for (ch, sf) in f.runs(s) {
            let id = sf.glyph_id(ch);
            if let Some((p, font)) = prev {
                if std::ptr::eq(font, sf.font) {
                    pen += sf.kern(p, id);
                }
            }
            if let Some(og) = sf.outline_glyph(id.with_scale_and_position(sf.scale, point(pen, baseline))) {
                let b = og.px_bounds();
                og.draw(|gx, gy, cov| self.blend(b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, c, cov));
            }
            pen += sf.h_advance(id);
            prev = Some((id, sf.font as *const _));
        }
    }

    pub(super) fn text(&mut self, x: f32, y: f32, s: &str, f: Font_, c: Rgb) {
        self.text_at(x, y, s, f, c, Anchor::La);
    }

    /// Letter-spaced uppercase label. right = anchor at the right end of the run.
    pub(super) fn caps(&mut self, x: f32, y: f32, text: &str, size: f32, fill: Option<Rgb>, spacing: f32, right: bool) -> f32 {
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
    pub(super) fn glow(&mut self, x0: u32, y0: u32, x1: u32, y1: u32) {
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
