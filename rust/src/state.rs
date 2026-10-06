//! Data shared between the collector threads and the renderer.

use std::collections::{HashMap, VecDeque};

use crate::ai::{HttpStatus, RateLimited, Usage};
use crate::hw::Hardware;
use crate::view::HIST_LEN;
use crate::weather::Weather;

pub const AI_NAMES: [&str; 3] = ["Claude", "Codex", "Antigravity"];

/// Why an AI value is missing or stale, reduced to what the screen shows.
#[derive(Clone, Debug)]
pub enum AiErr {
    RateLimited(f64),
    Http(u16),
    Other(String),
}

impl AiErr {
    pub fn from(e: &anyhow::Error) -> Self {
        if let Some(r) = e.downcast_ref::<RateLimited>() {
            AiErr::RateLimited(r.0)
        } else if let Some(h) = e.downcast_ref::<HttpStatus>() {
            AiErr::Http(h.0)
        } else if let Some(h) = e.downcast_ref::<ureq::Error>() {
            AiErr::Other(match h {
                ureq::Error::Timeout(_) => "Timeout".into(),
                ureq::Error::Io(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => "Network".into(),
                _ => "HTTP".into(),
            })
        } else {
            AiErr::Other("Error".into())
        }
    }
}

pub const HIST_KEYS: [&str; 13] = [
    "cpu", "ram", "gpu", "vram", "net_down", "net_up", "fan_radiator", "fan_pump", "fan_gpu",
    "cpu_temp", "ram_temp", "gpu_temp", "vram_temp",
];

#[derive(Default)]
pub struct State {
    pub hw: Hardware,
    pub weather: Option<Weather>,
    pub ai: HashMap<&'static str, Usage>,
    pub ai_err: HashMap<&'static str, AiErr>,
    /// One sample per second, oldest first (scrolling graphs)
    pub hist: HashMap<&'static str, VecDeque<Option<f64>>>,
    /// Per-core CPU usage, same layout as `hist`
    pub cores: Vec<VecDeque<Option<f64>>>,
}

impl State {
    pub fn new() -> Self {
        let mut s = State::default();
        for k in HIST_KEYS {
            s.hist.insert(k, VecDeque::with_capacity(HIST_LEN));
        }
        s
    }

    pub fn record(&mut self, hw: &Hardware) {
        let pct = |used: Option<f64>, total: Option<f64>| Some(used? / total? * 100.0);
        let values = [
            ("cpu", hw.cpu), ("ram", pct(hw.ram_used, hw.ram_total)), ("gpu", hw.gpu),
            ("vram", pct(hw.vram_used, hw.vram_total)), ("net_down", hw.net_down), ("net_up", hw.net_up),
            ("fan_radiator", hw.fan_radiator), ("fan_pump", hw.fan_pump), ("fan_gpu", hw.fan_gpu),
            ("cpu_temp", hw.cpu_temp), ("ram_temp", hw.ram_temp), ("gpu_temp", hw.gpu_temp),
            ("vram_temp", hw.vram_temp),
        ];
        for (k, v) in values {
            let h = self.hist.get_mut(k).unwrap();
            if h.len() == HIST_LEN {
                h.pop_front();
            }
            h.push_back(v);
        }
        self.cores.resize_with(hw.cpu_cores.len(), || VecDeque::with_capacity(HIST_LEN));
        for (h, &v) in self.cores.iter_mut().zip(&hw.cpu_cores) {
            if h.len() == HIST_LEN {
                h.pop_front();
            }
            h.push_back(Some(v));
        }
    }
}
