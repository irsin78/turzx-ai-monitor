//! Frames of the video the panel plays while the PC is off: the date and the month calendar on
//! the left, movies on the right, one after the other. Two kinds:
//! - stills: the backdrop slowly zooming, title, release, rating and story line on it, with a
//!   cross-fade (the last movie fades into the first, so the video loops without a seam);
//! - clips: video files (16:9) next to the calendar with the movie's details beside them,
//!   fading through the background between clips.

use tiny_skia::{GradientStop, LinearGradient, PixmapPaint, Point, Rect, SpreadMode};

use super::*;

/// Width of the date/calendar panel; the movies take the rest.
const LEFT: u32 = 540;
const RIGHT_W: u32 = W - LEFT;
pub const SLIDE_S: f32 = 6.0;
const FADE_S: f32 = 0.6;

pub struct Slide {
    pub title: String,
    pub now_playing: bool,
    /// "YYYY-MM-DD"
    pub release: String,
    pub rating: f32,
    pub overview: String,
    pub image: Pixmap,
}

/// The parts that stay the same for the whole video: background, date and calendar, credit.
pub fn offline_base(now: &DateTime<Local>) -> Canvas {
    let mut cv = Canvas::background();
    let t = i18n::get();
    let (x, w) = (48.0, LEFT as f32 - 96.0);
    let date = now.format_localized(t.date, t.locale).to_string();
    let f = fit_ko(&cv, &date, &[34.0, 30.0, 26.0, 22.0], false, w);
    cv.text(x, 46.0, &date, f, INK);
    draw_calendar(&mut cv, now, x, 128.0, w);
    cv.text(x, 436.0, "Movies: TMDB", ko(14.0, false), FAINT);
    cv
}

/// Seconds of video for `n` movies.
pub fn offline_len(n: usize) -> f32 {
    n as f32 * SLIDE_S
}

/// The frame at `t` seconds.
pub fn offline_frame(base: &Canvas, slides: &[Slide], t: f32) -> Canvas {
    let mut cv = base.clone();
    if slides.is_empty() {
        return cv;
    }
    let t = t.rem_euclid(offline_len(slides.len()));
    let i = ((t / SLIDE_S) as usize).min(slides.len() - 1);
    let local = t - i as f32 * SLIDE_S;
    draw_image(&mut cv, &slides[i], local, 1.0);
    // pictures cross-fade; the text of one movie is gone before the next one's appears
    let k = ((local - (SLIDE_S - FADE_S)) / FADE_S).clamp(0.0, 1.0);
    let next = &slides[(i + 1) % slides.len()];
    if k > 0.0 {
        draw_image(&mut cv, next, local - SLIDE_S, k);
    }
    draw_text(&mut cv, &slides[i], 1.0 - 2.0 * k);
    if k > 0.5 {
        draw_text(&mut cv, next, 2.0 * k - 1.0);
    }
    cv
}

/// A movie's picture, `local` seconds into its slide (negative while fading in), with `alpha`.
fn draw_image(cv: &mut Canvas, s: &Slide, local: f32, alpha: f32) {
    let (w, h) = (RIGHT_W as f32, H as f32);
    let mut sub = Canvas { pm: Pixmap::new(RIGHT_W, H).unwrap() };
    // backdrop: cover the area, zoom in slowly and drift sideways
    let p = ((local + FADE_S) / (SLIDE_S + FADE_S)).clamp(0.0, 1.0);
    let (iw, ih) = (s.image.width() as f32, s.image.height() as f32);
    let scale = (w / iw).max(h / ih) * (1.04 + 0.08 * p);
    let (sw, sh) = (iw * scale, ih * scale);
    let tx = (w - sw) * (0.35 + 0.3 * p);
    let ty = (h - sh) * 0.3;
    let paint = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
    sub.pm.draw_pixmap(0, 0, s.image.as_ref(), &paint, Transform::from_row(scale, 0.0, 0.0, scale, tx, ty), None);
    // darken the bottom for the text, then fade the left edge out so the panel shows through
    shade(&mut sub.pm, (0.0, h), (0.0, h * 0.42), [4, 6, 12], 0.88);
    let ramp = 260usize;
    let stride = RIGHT_W as usize * 4;
    for row in sub.pm.data_mut().chunks_exact_mut(stride) {
        for (x, px) in row[..ramp * 4].chunks_exact_mut(4).enumerate() {
            let a = (x as f32 / ramp as f32).powf(1.5); // premultiplied: scale every channel
            px.iter_mut().for_each(|v| *v = (*v as f32 * a) as u8);
        }
    }
    let paint = PixmapPaint { opacity: alpha.clamp(0.0, 1.0), ..Default::default() };
    cv.pm.draw_pixmap(LEFT as i32, 0, sub.pm.as_ref(), &paint, Transform::identity(), None);
}

/// A movie's title, release or "now showing", rating and story line, at `alpha`.
fn draw_text(cv: &mut Canvas, s: &Slide, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let w = RIGHT_W as f32;
    let x = LEFT as f32 + 64.0;
    let title_f = fit_ko(cv, &s.title, &[46.0, 40.0, 34.0, 30.0], true, w - 124.0);
    cv.text_faded(x, 300.0, &ellipsize(cv, &s.title, title_f, w - 124.0), title_f, INK, alpha);
    let info = release_line(s.now_playing, &s.release, s.rating);
    cv.text_faded(x, 368.0, &info, ko(24.0, false), ACCENT, alpha);
    if !s.overview.is_empty() {
        let f = ko(20.0, false);
        cv.text_faded(x, 412.0, &ellipsize(cv, &s.overview, f, w - 134.0), f, SOFT, alpha);
    }
}

/// "Opens Oct 28   ★ 7.2" / "Now showing   ★ 6.5", in the dashboard language.
fn release_line(now_playing: bool, release: &str, rating: f32) -> String {
    let t = i18n::get();
    let when = if now_playing {
        t.movies[0].to_string()
    } else {
        chrono::NaiveDate::parse_from_str(release, "%Y-%m-%d")
            .map(|d| t.movies[1].replace("{}", &d.format_localized(t.movie_date, t.locale).to_string()))
            .unwrap_or_default()
    };
    if rating > 0.0 { format!("{when}   ★ {rating:.1}") } else { when }
}

// --- clips ---------------------------------------------------------------------------------------

/// Where a clip goes: right of the calendar, 16:9 at the full height.
const CLIP_W: u32 = 853;
/// Fade through the background between clips, seconds each way.
pub const CLIP_FADE_S: f32 = 0.4;

/// A movie shown as a clip (the video comes separately, frame by frame).
pub struct Clip {
    pub title: String,
    pub now_playing: bool,
    /// "YYYY-MM-DD"
    pub release: String,
    pub rating: f32,
    pub overview: String,
}

/// Everything but the video for one clip: the calendar and the movie's details on the right.
pub fn clip_base(now: &DateTime<Local>, c: &Clip) -> Canvas {
    let mut cv = offline_base(now);
    let x = (LEFT + CLIP_W) as f32 + 34.0;
    let w = (W - LEFT - CLIP_W) as f32 - 68.0;
    let mut y = 52.0;
    let tf = ko(32.0, true);
    for line in wrap(&cv, &c.title, tf, w, 2) {
        cv.text(x, y, &line, tf, INK);
        y += 44.0;
    }
    y += 10.0;
    cv.text(x, y, &release_line(c.now_playing, &c.release, c.rating), ko(21.0, false), ACCENT);
    y += 50.0;
    let of = ko(18.0, false);
    let lines = ((440.0 - y) / 28.0).max(0.0) as usize;
    for line in wrap(&cv, &c.overview, of, w, lines) {
        cv.text(x, y, &line, of, SOFT);
        y += 28.0;
    }
    cv
}

/// One frame: `base` from `clip_base` with a video frame, faded in by `fade` (0..1).
pub fn clip_frame(base: &Canvas, frame: &Pixmap, fade: f32) -> Canvas {
    let mut cv = base.clone();
    let (fw, fh) = (frame.width() as f32, frame.height() as f32);
    let (w, h) = (CLIP_W as f32, H as f32);
    let scale = (w / fw).max(h / fh); // cover: a 16:9 video fits exactly
    let mut sub = Pixmap::new(CLIP_W, H).unwrap();
    let (tx, ty) = ((w - fw * scale) / 2.0, (h - fh * scale) / 2.0);
    let quality = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
    sub.draw_pixmap(0, 0, frame.as_ref(), &quality, Transform::from_row(scale, 0.0, 0.0, scale, tx, ty), None);
    cv.pm.draw_pixmap(LEFT as i32, 0, sub.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    if fade < 1.0 {
        // fade the movie part (video and details) through the background color
        let mut p = paint(BG);
        p.set_color_rgba8(BG[0], BG[1], BG[2], ((1.0 - fade.clamp(0.0, 1.0)) * 255.0) as u8);
        cv.pm.fill_rect(Rect::from_xywh(LEFT as f32, 0.0, (W - LEFT) as f32, h).unwrap(), &p, Transform::identity(), None);
    }
    cv
}

/// `text` broken into at most `max_lines` lines of `width` px (the last one cut with "…").
fn wrap(cv: &Canvas, text: &str, f: Font_, width: f32, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if cv.text_len(&candidate, f) <= width {
            line = candidate;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        line = word.to_string();
        // a word wider than the line (no spaces): break it by characters
        while cv.text_len(&line, f) > width {
            let mut head = String::new();
            for ch in line.chars() {
                if cv.text_len(&format!("{head}{ch}"), f) > width {
                    break;
                }
                head.push(ch);
            }
            line = line[head.len()..].to_string();
            lines.push(head);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            *last = ellipsize(cv, &format!("{last}…"), f, width);
        }
    }
    lines
}

/// A gradient from `color` at `from` (opacity `a`) to transparent at `to`.
fn shade(pm: &mut Pixmap, from: (f32, f32), to: (f32, f32), color: Rgb, a: f32) {
    let c = |alpha: f32| tiny_skia::Color::from_rgba8(color[0], color[1], color[2], (alpha * 255.0) as u8);
    let stops = vec![GradientStop::new(0.0, c(a)), GradientStop::new(1.0, c(0.0))];
    let Some(shader) = LinearGradient::new(Point::from_xy(from.0, from.1), Point::from_xy(to.0, to.1), stops, SpreadMode::Pad, Transform::identity()) else { return };
    let paint = tiny_skia::Paint { shader, ..Default::default() };
    let rect = Rect::from_xywh(0.0, 0.0, pm.width() as f32, pm.height() as f32).unwrap();
    pm.fill_rect(rect, &paint, Transform::identity(), None);
}

/// `s` cut to `max` px with "…".
fn ellipsize(cv: &Canvas, s: &str, f: Font_, max: f32) -> String {
    if cv.text_len(s, f) <= max {
        return s.to_string();
    }
    let mut out: String = String::new();
    for ch in s.chars() {
        if cv.text_len(&format!("{out}{ch}…"), f) > max {
            break;
        }
        out.push(ch);
    }
    format!("{}…", out.trim_end())
}
