//! Column B: CPU / memory / GPU / VRAM with graphs and temperatures, network; column C top: fans.

use super::*;

/// Scrolling area chart, newest at the right edge, 0-100 %.
pub(super) fn graph(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, samples: &VecDeque<Option<f64>>, hot: bool) {
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
pub(super) fn bg_graph(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, samples: &VecDeque<Option<f64>>, floor: f64, span: Option<f64>) {
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
pub(super) fn core_graphs(cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, cores: &[VecDeque<Option<f64>>]) {
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

pub(super) fn draw_hardware(cv: &mut Canvas, st: &State, x: f32, right: f32) {
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
            Some(t) => cv.text(tx, y + 24.0, &format!("{:.0}°", deg(t)), tf, temp_color(t)),
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

pub(super) fn draw_fans(cv: &mut Canvas, st: &State, x: f32, right: f32) {
    cv.caps(x, TOP, "Fans", 16.0, None, 2.5, false);
    cv.caps(right, TOP, "RPM", 14.0, Some(MUTED), 2.0, true);
    let hw = &st.hw;
    // names from the settings, or the language's defaults
    let custom = &config::get().fans.labels;
    let name = |i: usize| custom.get(i).map_or(i18n::get().fans[i], String::as_str);
    let fans = [(name(0), "fan_radiator", hw.fan_radiator), (name(1), "fan_pump", hw.fan_pump), (name(2), "fan_gpu", hw.fan_gpu)];
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
