//! Weather glyphs (solid, monochrome).

use super::*;

pub(super) fn weather_icon(cv: &mut Canvas, cx: f32, cy: f32, kind: &str, s: f32) {
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
