//! Frames of the video the panel plays while the PC is off: the date and the month calendar on
//! the left, movies on the right, one after the other. Two kinds:
//! - stills: the backdrop slowly zooming, title, release, rating and story line on it, with a
//!   cross-fade (the last movie fades into the first, so the video loops without a seam);
//! - clips: video files (16:9) next to the calendar with the movie's details beside them,
//!   fading through the background between clips.

use tiny_skia::{GradientStop, LinearGradient, PixmapPaint, Point, Rect, SpreadMode};

use super::*;

/// Width of the date/calendar panel; the movies take the rest.
const LEFT: u32 = 610;
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

/// The parts that stay the same for the whole video: background, date and calendar, today's
/// and tomorrow's weather, credit.
pub fn offline_base(now: &DateTime<Local>, weather: Option<&crate::sources::weather::Weather>) -> Canvas {
    offline_base_days(now, weather, crate::sources::holidays::month(now.year(), now.month()))
}

fn offline_base_days(now: &DateTime<Local>, weather: Option<&Weather>, holidays: u32) -> Canvas {
    let mut cv = Canvas::background();
    cv.rect(0.0, 0.0, LEFT as f32, H as f32, [17, 25, 36]);
    let t = i18n::get();
    let (x, right) = COL_A;
    let cal_x = right - 210.0;
    let width = cal_x - 24.0 - x;
    let date = now.format_localized(t.date, t.locale).to_string();
    let mut f = ko(22.0, false);
    for size in [38.0, 34.0, 30.0, 26.0, 22.0] {
        f = ko(size, false);
        if wrap(&cv, &date, f, width, usize::MAX).len() <= 3 {
            break;
        }
    }
    for (i, line) in wrap(&cv, &date, f, width, 3).iter().enumerate() {
        cv.text(x, 64.0 + i as f32 * 50.0, line, f, INK);
    }
    draw_calendar_days(&mut cv, now, cal_x, TOP, 210.0, holidays);
    cv.text(x, 236.0, "Movies: TMDB", ko(12.0, false), FAINT);
    if weather.is_some() {
        let hm = now.format(if config::get().hour24() { "%H:%M" } else { "%-I:%M %p" }).to_string();
        let stamp = fill(t.weather_as_of, hm);
        let f = fit_ko(&cv, &stamp, &[14.0, 13.0, 12.0], false, 300.0);
        cv.text(right - cv.text_len(&stamp, f), 236.0, &stamp, f, MUTED);
    }
    cv.line(&[(x, 262.0), (right, 262.0)], 1.0, RULE);
    draw_weather(&mut cv, weather, x, 284.0, right);
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
pub fn clip_base(now: &DateTime<Local>, weather: Option<&crate::sources::weather::Weather>, c: &Clip) -> Canvas {
    let mut cv = offline_base(now, weather);
    draw_clip_details(&mut cv, c);
    cv
}

fn draw_clip_details(cv: &mut Canvas, c: &Clip) {
    let x = (LEFT + CLIP_W) as f32 + 34.0;
    let w = (W - LEFT - CLIP_W) as f32 - 68.0;
    let mut y = 52.0;
    let tf = ko(32.0, true);
    for line in wrap(cv, &c.title, tf, w, 2) {
        cv.text(x, y, &line, tf, INK);
        y += 44.0;
    }
    y += 10.0;
    cv.text(x, y, &release_line(c.now_playing, &c.release, c.rating), ko(21.0, false), ACCENT);
    y += 50.0;
    let of = ko(18.0, false);
    let lines = ((440.0 - y) / 28.0).max(0.0) as usize;
    for line in wrap(cv, &c.overview, of, w, lines) {
        cv.text(x, y, &line, of, SOFT);
        y += 28.0;
    }
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

/// Synthetic clip/still previews, including the missing-weather paths, without network or USB.
pub fn offline_preview() -> (Canvas, Canvas) {
    use chrono::{NaiveTime, TimeZone};

    let now = Local.with_ymd_and_hms(2026, 11, 16, 15, 42, 0).single().unwrap();
    let weather = Weather {
        temp: 18.0, feels: 17.0, humidity: 62, code: 2, night: false,
        t_min: 11.0, t_max: 21.0, rain_prob: Some(30),
        tomorrow: Some(wx::Forecast { code: 61, t_min: 10.0, t_max: 19.0, rain_prob: Some(80) }),
        sunrise: NaiveTime::from_hms_opt(7, 12, 0), sunset: NaiveTime::from_hms_opt(17, 23, 0),
        pm25: Some(12.0), pm10: Some(24.0), aqi25: Some(50.0), aqi10: Some(22.0),
    };
    let clip = Clip {
        title: "The Last Observatory Beyond the Edge of Tomorrow: A Journey Through the Stars".into(),
        now_playing: false, release: "2026-11-28".into(), rating: 8.2,
        overview: "When a forgotten observatory receives a signal from the far side of the galaxy, a young astronomer and an unlikely crew set out to discover who sent it. Their journey across distant worlds reveals a mystery that could change the future of everyone they left behind.".into(),
    };
    let gradient = |w: u32, h: u32, backdrop: bool| {
        let mut pm = Pixmap::new(w, h).unwrap();
        for (i, px) in pm.pixels_mut().iter_mut().enumerate() {
            let (x, y) = (i as u32 % w, i as u32 / w);
            let (a, b) = ((x * 160 / (w - 1)) as u8, (y * 120 / (h - 1)) as u8);
            let c = if backdrop { [30 + b, 45 + a, 100 + b] } else { [35 + a, 55 + b, 160 - b] };
            *px = tiny_skia::PremultipliedColorU8::from_rgba(c[0], c[1], c[2], 255).unwrap();
        }
        pm
    };
    let frame = gradient(1280, 720, false);
    let slides = [Slide {
        title: clip.title.clone(), now_playing: clip.now_playing, release: clip.release.clone(),
        rating: clip.rating, overview: clip.overview.clone(), image: gradient(1600, 900, true),
    }];
    // Supply holiday bits directly so the calendar cannot start a background holiday fetch.
    let base = offline_base_days(&now, Some(&weather), 0);
    let mut clip_base = base.clone();
    draw_clip_details(&mut clip_base, &clip);
    let preview_clip = clip_frame(&clip_base, &frame, 1.0);
    let preview_still = offline_frame(&base, &slides, 3.0);
    let mut empty = offline_base_days(&now, None, 0);
    let _ = offline_frame(&empty, &slides, 3.0);
    draw_clip_details(&mut empty, &clip);
    let _ = clip_frame(&empty, &frame, 1.0);
    (preview_clip, preview_still)
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
