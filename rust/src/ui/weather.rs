//! Column A, bottom: weather now, air quality, sunrise / sunset and tomorrow.

use super::*;

pub(super) fn air_color(a: Air) -> Rgb {
    // AirKorea palette, softened for the dark theme
    match a {
        Air::Good => [110, 170, 255],
        Air::Moderate => [110, 205, 140],
        Air::Bad => [236, 190, 80],
        Air::VeryBad => [236, 120, 80],
        Air::Hazardous | Air::Watch | Air::Warning => HOT,
    }
}

fn air_word(a: Air) -> &'static str {
    i18n::get().air[a as usize]
}

/// "PM2.5 7 Good   PM10 8 Good": value plus its grade in color (Korean grades or US AQI, see
/// the settings), shrunk to fit `right`.
pub(super) fn draw_air(cv: &mut Canvas, w: &Weather, x: f32, y: f32, right: f32) {
    let t = i18n::get();
    let korean = config::get().korean_air_scale();
    let grade = |pm: Option<f64>, aqi: Option<f64>, fine: bool| if korean { pm.map(|v| wx::air_korea(v, fine)) } else { aqi.map(wx::air_us) };
    let items: Vec<(&str, Option<String>, Option<Air>)> = [(t.pm25, w.pm25, w.aqi25, true), (t.pm10, w.pm10, w.aqi10, false)]
        .into_iter()
        .map(|(label, pm, aqi, fine)| (label, pm.map(|v| format!("{v:.0}")), grade(pm, aqi, fine)))
        .collect();
    // with grade words while they fit, then just the numbers colored by grade
    let tries = [19.0, 18.0, 17.0, 16.0, 15.0, 14.0].map(|z| (z, true)).into_iter().chain([18.0, 16.0, 14.0].map(|z| (z, false)));
    for (size, words) in tries {
        let (f_lab, f_val, f_grade) = (ko(size, false), num(size + 4.0, Face::Num), ko(size, true));
        let (gap, sep) = (size * 0.3, size * 1.0);
        let width: f32 = items
            .iter()
            .map(|(label, v, g)| {
                cv.text_len(label, f_lab) + gap + cv.text_len(v.as_deref().unwrap_or("--"), f_val)
                    + g.filter(|_| words).map_or(0.0, |a| gap + cv.text_len(air_word(a), f_grade))
            })
            .sum::<f32>()
            + sep;
        if x + width > right && !(size == 14.0 && !words) {
            continue;
        }
        let mut cx = x;
        for (label, v, g) in &items {
            cv.text(cx, y, label, f_lab, SOFT);
            cx += cv.text_len(label, f_lab) + gap;
            let v = v.as_deref().unwrap_or("--");
            let vc = match g {
                None => FAINT,
                Some(a) if !words => air_color(*a),
                Some(_) => INK,
            };
            cv.text(cx, y - 1.0, v, f_val, vc);
            cx += cv.text_len(v, f_val);
            if let (Some(a), true) = (g, words) {
                cx += gap;
                cv.text(cx, y, air_word(*a), f_grade, air_color(*a));
                cx += cv.text_len(air_word(*a), f_grade);
            }
            cx += sep;
        }
        break;
    }
}

/// Sunrise / sunset (12- or 24-hour, as the clock) with small up/down glyphs, right-aligned to `right`.
pub(super) fn draw_sun(cv: &mut Canvas, w: &Weather, min_x: f32, y: f32, right: f32) {
    let (Some(rise), Some(set)) = (w.sunrise, w.sunset) else { return };
    let h24 = config::get().hour24();
    let hm = |t: chrono::NaiveTime| {
        let h = if h24 { t.hour() } else if t.hour() % 12 == 0 { 12 } else { t.hour() % 12 };
        format!("{}:{:02}", h, t.minute())
    };
    let (a, b) = (hm(rise), hm(set));
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

pub(super) const TOMORROW_W: f32 = 145.0; // tomorrow block, from its left edge to the column's right edge
pub(super) const TOMORROW_ICON_R: f32 = 20.0; // half the width of the small weather icon

pub(super) fn draw_weather(cv: &mut Canvas, st: &State, x: f32, y: f32, right: f32) {
    let s = i18n::get();
    let Some(w) = &st.weather else {
        cv.text(x, y + 20.0, s.loading, ko(22.0, false), MUTED);
        return;
    };
    let dg = |c: f64| format!("{:.0}°", deg(c));
    weather_icon(cv, x + 39.0, y + 58.0, wx::icon(w.code, w.night), 0.83);
    let big = num(69.0, Face::NumLight);
    let t = dg(w.temp);
    cv.text(x + 96.0, y + 14.0, &t, big, INK);
    let tx = x + 96.0 + cv.text_len(&t, big) + 22.0;
    let limit = right - TOMORROW_W - 36.0 - tx; // room left of the tomorrow block
    let label = s.weather_label(w.code);
    cv.text(tx, y + 26.0, label, fit_ko(cv, label, &[28.0, 26.0, 24.0, 22.0, 20.0, 18.0], true, limit), INK);
    let line = format!("{}  ·  {}", fill(s.feels, dg(w.feels)), fill(s.humidity, w.humidity));
    cv.text(tx, y + 63.0, &line, fit_ko(cv, &line, &[18.0, 17.0, 16.0], false, limit), SOFT);
    draw_air(cv, w, tx, y + 91.0, right - TOMORROW_W - 36.0);
    let rain = w.rain_prob.map(|p| format!("  ·  {}", fill(s.rain, p))).unwrap_or_default();
    let minmax = format!("{}  ·  {}{rain}", fill(s.low, dg(w.t_min)), fill(s.high, dg(w.t_max)));
    let f_mm = fit_ko(cv, &minmax, &[19.0, 18.0, 17.0], false, right - TOMORROW_W - 40.0 - x);
    cv.text(x, y + 124.0, &minmax, f_mm, MUTED);
    draw_sun(cv, w, x, y + 150.0, right - TOMORROW_W - 40.0);

    if let Some(t) = &w.tomorrow {
        let tx = right - TOMORROW_W;
        cv.line(&[(tx - 24.0, y + 6.0), (tx - 24.0, y + 150.0)], 1.0, RULE);
        if s.tomorrow_caps {
            cv.caps(tx, y + 4.0, s.tomorrow, 14.0, Some(MUTED), 2.0, false);
        } else {
            cv.text(tx, y + 1.0, s.tomorrow, ko(16.0, false), MUTED);
        }
        // icon and label sit a little left of the temperatures below
        weather_icon(cv, tx + TOMORROW_ICON_R - 5.0, y + 58.0, wx::icon(t.code, false), 0.5);
        let (lx, label) = (tx + TOMORROW_ICON_R * 2.0 + 5.0, s.weather_label(t.code));
        cv.text(lx, y + 44.0, label, fit_ko(cv, label, &[18.0, 17.0, 16.0, 15.0, 14.0, 13.0], false, RULES_X[0] - 12.0 - lx), INK);
        for (i, (template, temp)) in [(s.low, t.t_min), (s.high, t.t_max)].into_iter().enumerate() {
            let x = tx + i as f32 * TOMORROW_W / 2.0;
            let label = fill(template, "").trim().to_owned();
            let font = fit_ko(cv, &label, &[15.0, 14.0, 13.0, 12.0], false, TOMORROW_W / 2.0 - 6.0);
            cv.text(x, y + 96.0, &label, font, SOFT);
            cv.text(x, y + 113.0, &dg(temp), num(26.0, Face::Num), INK);
        }
        if let Some(p) = t.rain_prob {
            let rain = fill(s.rain, p);
            let font = fit_ko(cv, &rain, &[19.0, 18.0, 17.0, 16.0], true, TOMORROW_W);
            cv.text(right - cv.text_len(&rain, font), y + 151.0, &rain, font, SOFT);
        }
    }
}
