//! Diagnostics and previews: `turzx-dashboard diag <command>`.

use std::time::Duration;

use anyhow::{bail, Result};

use crate::panel::{jpeg, lcd};
use crate::sensors::{self as hw};
use crate::sources::{ai, weather};
use crate::{mascot, state, ui};

pub const COMMANDS: &str = "sensors | weather | claude-cli | layout | standby | mascots | jpeg-check | profile | fps-test | test-pattern";

/// Four vertical quarter bands (red, green, blue, white).
fn test_bands() -> Vec<u8> {
    let (w, h) = (lcd::HEIGHT, lcd::WIDTH); // landscape 1920x480
    let colors = [[200, 0, 0], [0, 160, 0], [0, 0, 200], [230, 230, 230]];
    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let c = colors[x * 4 / w];
            rgb[(y * w + x) * 3..][..3].copy_from_slice(&c);
        }
    }
    rgb
}

/// Dummy AI values and graph history for --layout previews (mirrors dashboard._fake_data).
pub fn fake_state() -> state::State {
    let mut st = state::State::new();
    let now = ai::now();
    let u = |s, sr, w, wr| ai::Usage { session: Some(s), session_reset: Some(now + sr), weekly: Some(w), weekly_reset: Some(now + wr), source: "fake".into() };
    st.ai.insert("Claude", u(12.0, 8000.0, 45.0, 300000.0));
    st.ai.insert("Codex", u(3.0, 17000.0, 37.0, 270000.0));
    st.ai.insert("Antigravity", u(92.0, 3000.0, 0.1, 560000.0));
    st.hw = hw::Hardware {
        cpu: Some(12.0), cpu_cores: vec![0.0; 14], cpu_temp: Some(46.0), cpu_power: Some(38.0), ram_used: Some(14.1), ram_total: Some(64.0), ram_temp: Some(46.5),
        gpu: Some(4.0), gpu_temp: Some(37.0), gpu_power: Some(24.0), vram_used: Some(2.7), vram_total: Some(15.9), vram_temp: Some(48.0),
        fan_radiator: Some(872.0), fan_pump: Some(3392.0), fan_gpu: Some(1167.0),
        net_down: Some(49_200.0 / 8.0), net_up: Some(29_000.0 / 8.0),
    };
    let mut seed = 12345u32;
    let mut rnd = move || {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        (seed >> 16) as f64 / 65535.0 * 2.0 - 1.0
    };
    let series: [(&str, f64, f64, f64, f64); 13] = [
        // key, base, bump amplitude, bump center, noise
        ("cpu", 15.0, 55.0, 70.0, 4.0), ("ram", 22.0, 0.0, 70.0, 4.0), ("gpu", 30.0, 55.0, 70.0, 4.0), ("vram", 16.0, 0.0, 70.0, 4.0),
        ("net_down", 3e5, 4e6, 75.0, 1.5e4), ("net_up", 5e4, 6e5, 75.0, 2.5e3), ("fan_radiator", 880.0, 400.0, 75.0, 44.0),
        ("fan_pump", 3390.0, 30.0, 75.0, 170.0), ("fan_gpu", 1165.0, 500.0, 75.0, 58.0), ("cpu_temp", 44.0, 18.0, 75.0, 2.2),
        ("ram_temp", 46.0, 2.0, 75.0, 2.3), ("gpu_temp", 36.0, 14.0, 75.0, 1.8), ("vram_temp", 46.0, 6.0, 75.0, 2.3),
    ];
    for (k, base, amp, center, noise) in series {
        let h = st.hist.get_mut(k).unwrap();
        for i in 0..ui::HIST_LEN {
            let bump = amp * (-((i as f64 - center) / if k.len() <= 4 { 8.0 } else { 10.0 }).powi(2)).exp();
            let v = base + bump + rnd() * noise;
            h.push_back(Some(if k.len() <= 4 { v.min(100.0) } else { v.max(0.0) }));
        }
    }
    // per-core: a few busy P-cores during the bump, idle E-cores with noise
    for c in 0..14 {
        let busy = if c < 6 { 70.0 - c as f64 * 10.0 } else { 15.0 };
        st.cores.push((0..ui::HIST_LEN).map(|i| {
            let bump = busy * (-((i as f64 - 110.0) / 6.0).powi(2)).exp();
            Some((8.0 + bump + rnd() * 6.0).clamp(0.0, 100.0))
        }).collect());
    }
    st.weather = weather::fetch().map_err(|e| log::warn!("weather: {e}")).ok();
    st
}


pub fn run(cmd: &str) -> Result<()> {
    match cmd {
        "sensors" => {
            let mut r = hw::HardwareReader::new();
            for _ in 0..3 {
                std::thread::sleep(Duration::from_secs(1));
                println!("{:?}", r.read());
            }
    
        }
        "weather" => {
            println!("{:#?}", weather::fetch()?);
    
        }
        "claude-cli" => {
            let t = std::time::Instant::now();
            println!("{:?} in {:?}", ai::ClaudeCli.fetch(Duration::from_secs(20)), t.elapsed());
    
        }
        "layout" => {
            let st = fake_state();
            let t = std::time::Instant::now();
            let cv = ui::render(&st, chrono::Local::now());
            let dt = t.elapsed();
            cv.save_png("preview_rust.png")?;
            println!("rendered in {dt:?}, saved preview_rust.png");
    
        }
        "standby" => {
            for i in 0..3 {
                ui::standby_page(i).save_png(&format!("preview_standby_{}.png", i + 1))?;
            }
            println!("saved preview_standby_1..3.png");
    
        }
        "mascots" => {
            // 30 s of the AI block at 12 fps as PNG frames (preview: frames -> GIF)
            let st = fake_state();
            let base = ui::render(&st, chrono::Local::now());
            let areas = ui::mascot_areas();
            let mut m = mascot::Mascots::new(&areas);
            std::fs::create_dir_all("mascot_frames")?;
            for i in 0..360 {
                m.update(1.0 / 12.0, &areas);
                let mut cv = base.clone();
                m.draw(cv.pixmap_mut(), &areas);
                cv.save_png(&format!("mascot_frames/{i:03}.png"))?;
            }
            println!("saved mascot_frames/000..359.png");
    
        }
        "jpeg-check" => {
            // Encode the preview with the band encoder: full, then re-encode some bands; save both
            let st = fake_state();
            let cv = ui::render(&st, chrono::Local::now());
            cv.save_png("jpeg_check_src.png")?;
            let mut enc = jpeg::RowJpeg::new();
            let t = std::time::Instant::now();
            enc.encode(cv.pixmap(), 0..jpeg::BANDS);
            let full_ms = t.elapsed().as_secs_f64() * 1000.0;
            let full = enc.frame().to_vec();
            let band = jpeg::bands_for(1300.0, 1560.0);
            let t = std::time::Instant::now();
            enc.encode(cv.pixmap(), band.clone());
            let part_ms = t.elapsed().as_secs_f64() * 1000.0;
            let again = enc.frame().to_vec();
            std::fs::write("jpeg_check.jpg", &full)?;
            #[cfg(target_arch = "x86_64")]
            {
                let f = |n: &str, on: bool| if on { n.to_string() } else { format!("-{n}") };
                println!("cpu: {} {} {} {} {} {} | compiled with avx2: {}", f("sse4.2", is_x86_feature_detected!("sse4.2")), f("avx", is_x86_feature_detected!("avx")),
                    f("avx2", is_x86_feature_detected!("avx2")), f("fma", is_x86_feature_detected!("fma")), f("avx512f", is_x86_feature_detected!("avx512f")),
                    f("bmi2", is_x86_feature_detected!("bmi2")), cfg!(target_feature = "avx2"));
            }
            println!("full encode {full_ms:.2} ms, {} KB; bands {band:?} re-encoded in {part_ms:.2} ms; identical: {}", full.len() / 1024, full == again);
    
        }
        "profile" => {
            // Per-frame CPU of the render loop, stage by stage, without the panel (same steps as app::run)
            let st = fake_state();
            let areas = ui::mascot_areas();
            let mut m = mascot::Mascots::new(&areas);
            let mut enc = jpeg::RowJpeg::new();
            let band = jpeg::bands_for(areas.iter().map(|a| a.x0).fold(f32::MAX, f32::min) - 64.0, areas.iter().map(|a| a.x1).fold(0.0, f32::max) + 64.0);
            let cols = band.start * 8..band.end * 8;
            let n = 240u32;
            let names = ["redraw+full encode (1/s)", "restore band", "mascots", "encode band", "assemble frame"];
            let mut t = [Duration::ZERO; 5];
            let mut base = ui::render(&st, chrono::Local::now());
            let mut work = base.clone();
            for i in 0..n {
                let s0 = std::time::Instant::now();
                let fresh = i % 12 == 0;
                if fresh {
                    base = ui::render(&st, chrono::Local::now());
                    work = base.clone();
                    enc.encode(work.pixmap(), 0..jpeg::BANDS);
                }
                let s1 = std::time::Instant::now();
                if !fresh {
                    work.restore_columns(&base, cols.clone());
                }
                let s2 = std::time::Instant::now();
                m.update(1.0 / 12.0, &areas);
                m.draw(work.pixmap_mut(), &areas);
                let s3 = std::time::Instant::now();
                if !fresh {
                    enc.encode(work.pixmap(), band.clone());
                }
                let s4 = std::time::Instant::now();
                let _ = enc.frame().len();
                let s5 = std::time::Instant::now();
                for (k, d) in [s1 - s0, s2 - s1, s3 - s2, s4 - s3, s5 - s4].into_iter().enumerate() {
                    t[k] += d;
                }
            }
            let total: Duration = t.iter().sum();
            println!("bands {band:?} ({} of {})", band.len(), jpeg::BANDS);
            for (k, d) in t.iter().enumerate() {
                println!("{:<26} {:6.2} ms/frame", names[k], d.as_secs_f64() * 1000.0 / n as f64);
            }
            let per = total.as_secs_f64() * 1000.0 / n as f64;
            println!("{:<26} {:6.2} ms/frame -> {:.1}% of one core at 12 fps", "total", per, per * 12.0 / 10.0);
    
        }
        "fps-test" => {
            // How many full frames per second the panel takes: rotate / encode / send timed apart
            let mut panel = lcd::Lcd::open()?;
            let base = ui::render(&fake_state(), chrono::Local::now()).rgb();
            let (w, h) = (lcd::HEIGHT, lcd::WIDTH);
            let (mut t_rot, mut t_enc, mut t_send, mut bytes) = (Duration::ZERO, Duration::ZERO, Duration::ZERO, 0usize);
            let start = std::time::Instant::now();
            let mut n = 0u32;
            while start.elapsed() < Duration::from_secs(10) {
                let mut rgb = base.clone();
                // a moving block so every frame differs
                let bx = (n as usize * 12) % (w - 60);
                for y in 200..260 {
                    for x in bx..bx + 60 {
                        rgb[(y * w + x) * 3..][..3].copy_from_slice(&[217, 119, 87]);
                    }
                }
                let t = std::time::Instant::now();
                let rot = lcd::rotate_cw(&rgb, w, h);
                t_rot += t.elapsed();
                let t = std::time::Instant::now();
                let jpeg = lcd::encode_jpeg(&rot, lcd::WIDTH, lcd::HEIGHT)?;
                t_enc += t.elapsed();
                bytes += jpeg.len();
                let t = std::time::Instant::now();
                panel.send_jpeg(&jpeg)?;
                t_send += t.elapsed();
                n += 1;
            }
            let secs = start.elapsed().as_secs_f64();
            let ms = |d: Duration| d.as_secs_f64() * 1000.0 / n as f64;
            println!(
                "{n} frames in {secs:.1}s = {:.1} fps | per frame: rotate {:.1} ms, encode {:.1} ms, send {:.1} ms | jpeg {:.0} KB",
                n as f64 / secs, ms(t_rot), ms(t_enc), ms(t_send), bytes as f64 / n as f64 / 1024.0
            );
    
        }
        "test-pattern" => {
            let mut lcd = lcd::Lcd::open()?;
            println!("firmware: {}", lcd.firmware);
            let t = std::time::Instant::now();
            lcd.show_landscape(&test_bands())?;
            println!("sent test pattern in {:?}", t.elapsed());
    
        }
        _ => bail!("unknown diag command {cmd:?}; one of: {COMMANDS}"),
    }
    Ok(())
}
