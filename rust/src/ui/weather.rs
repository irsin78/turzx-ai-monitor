//! Column A, bottom: weather now, air quality, sunrise / sunset and tomorrow.

use super::*;

pub(super) fn air_color(level: u8) -> Rgb {
    // AirKorea palette, softened for the dark theme
    [[110, 170, 255], [110, 205, 140], [236, 190, 80], [236, 120, 80], HOT][level as usize]
}

/// "초미세 7 좋음   미세 8 좋음": value plus the Korean grade in its color, shrunk to fit `right`.
pub(super) fn draw_air(cv: &mut Canvas, w: &Weather, x: f32, y: f32, right: f32) {
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
pub(super) fn draw_sun(cv: &mut Canvas, w: &Weather, min_x: f32, y: f32, right: f32) {
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

pub(super) const TOMORROW_W: f32 = 122.0; // tomorrow block, from its left edge to the column's right edge
pub(super) const TOMORROW_ICON_R: f32 = 20.0; // half the width of the small weather icon

pub(super) fn draw_weather(cv: &mut Canvas, st: &State, x: f32, y: f32, right: f32) {
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
