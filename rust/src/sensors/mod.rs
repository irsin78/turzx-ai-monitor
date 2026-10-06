//! Hardware readings: sysinfo (CPU, RAM, network), NVML (GPU), NvAPI (VRAM temperature),
//! PawnIO (CPU/RAM temperatures, board fans; administrator only).

use std::time::Instant;

use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use nvml_wrapper::Nvml;
use sysinfo::{Networks, System};

mod nvapi;
pub mod pawnio;

use nvapi::NvApiThermal;

#[derive(Clone, Debug, Default)]
pub struct Hardware {
    pub cpu: Option<f64>,
    pub cpu_cores: Vec<f64>, // per logical core, %
    pub cpu_temp: Option<f64>,
    pub cpu_power: Option<f64>, // W, package (RAPL)
    pub ram_used: Option<f64>, // GiB
    pub ram_total: Option<f64>,
    pub ram_temp: Option<f64>,
    pub gpu: Option<f64>,
    pub gpu_temp: Option<f64>,
    pub gpu_power: Option<f64>, // W, board
    pub vram_used: Option<f64>, // GiB
    pub vram_total: Option<f64>,
    pub vram_temp: Option<f64>,
    pub fan_radiator: Option<f64>, // RPM
    pub fan_pump: Option<f64>,
    pub fan_gpu: Option<f64>,
    pub net_down: Option<f64>, // bytes/s
    pub net_up: Option<f64>,
}

/// Physically installed memory in bytes (GetPhysicallyInstalledSystemMemory reports KiB).
fn installed_ram() -> Option<f64> {
    let mut kib = 0u64;
    unsafe { windows::Win32::System::SystemInformation::GetPhysicallyInstalledSystemMemory(&mut kib) }.ok()?;
    (kib > 0).then(|| kib as f64 * 1024.0)
}

pub struct HardwareReader {
    sys: System,
    nets: Networks,
    net_prev: Option<Instant>,
    nvml: Option<Nvml>,
    nvapi: Option<NvApiThermal>,
    cpu_temp: Option<pawnio::IntelCpuTemp>,
    fans: Option<pawnio::NuvotonFans>,
    dimms: Option<pawnio::Ddr5Temps>,
    ram_temp: Option<f64>,
    ram_next: Instant,
    energy_prev: Option<(u32, Instant)>,
}

// Adapters whose traffic is already counted on a physical NIC, or is not real network
const NET_SKIP: [&str; 8] = ["loopback", "vethernet", "virtual", "vmware", "virtualbox", "bluetooth", "docker", "wsl"];

impl HardwareReader {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_usage(); // prime the counter; the first reading is always 0
        let nvml = Nvml::init().map_err(|e| log::warn!("NVML unavailable: {e}")).ok();
        let nvapi = NvApiThermal::new().map_err(|e| log::warn!("NvAPI unavailable: {e}")).ok();
        let (mut cpu_temp, mut fans, mut dimms) = (None, None, None);
        match pawnio::library() {
            Ok(lib) => {
                cpu_temp = pawnio::IntelCpuTemp::new(&lib).map_err(|e| log::warn!("PawnIO IntelCpuTemp unavailable: {e}")).ok();
                fans = pawnio::NuvotonFans::new(&lib).map_err(|e| log::warn!("PawnIO NuvotonFans unavailable: {e}")).ok();
                dimms = pawnio::Ddr5Temps::new(&lib).map_err(|e| log::warn!("PawnIO Ddr5Temps unavailable: {e}")).ok();
            }
            Err(e) => log::warn!("PawnIO unavailable: {e}"),
        }
        HardwareReader {
            sys,
            nets: Networks::new_with_refreshed_list(),
            net_prev: None,
            nvml,
            nvapi,
            cpu_temp,
            fans,
            dimms,
            ram_temp: None,
            ram_next: Instant::now(),
            energy_prev: None,
        }
    }

    fn net_rates(&mut self) -> (Option<f64>, Option<f64>) {
        self.nets.refresh(true);
        let now = Instant::now();
        let prev = self.net_prev.replace(now);
        let (mut rx, mut tx) = (0u64, 0u64);
        for (name, data) in self.nets.iter() {
            if NET_SKIP.iter().any(|k| name.to_lowercase().contains(k)) {
                continue;
            }
            rx += data.received();
            tx += data.transmitted();
        }
        match prev {
            Some(p) => {
                let dt = (now - p).as_secs_f64().max(1e-3);
                (Some(rx as f64 / dt), Some(tx as f64 / dt))
            }
            None => (None, None), // first call: counters start now
        }
    }

    pub fn read(&mut self) -> Hardware {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        let gib = 1024.0 * 1024.0 * 1024.0;
        let total = self.sys.total_memory() as f64;
        let mut hw = Hardware {
            cpu: Some(self.sys.global_cpu_usage() as f64),
            cpu_cores: self.sys.cpus().iter().map(|c| c.cpu_usage() as f64).collect(),
            ram_used: Some((total - self.sys.available_memory() as f64) / gib),
            // installed DIMM capacity (64), not what Windows can use after hardware reservations (63.5)
            ram_total: Some(installed_ram().unwrap_or(total) / gib),
            ..Default::default()
        };
        (hw.net_down, hw.net_up) = self.net_rates();

        if let Some(dev) = self.nvml.as_ref().and_then(|n| n.device_by_index(0).ok()) {
            hw.gpu = dev.utilization_rates().ok().map(|u| u.gpu as f64);
            hw.gpu_temp = dev.temperature(TemperatureSensor::Gpu).ok().map(|t| t as f64);
            hw.gpu_power = dev.power_usage().ok().map(|mw| mw as f64 / 1000.0);
            if let Ok(m) = dev.memory_info() {
                hw.vram_used = Some(m.used as f64 / gib);
                hw.vram_total = Some(m.total as f64 / gib);
            }
            hw.fan_gpu = dev
                .num_fans()
                .ok()
                .and_then(|n| (0..n).filter_map(|i| dev.fan_speed_rpm(i).ok()).max())
                .map(|r| r as f64);
        }
        hw.vram_temp = self.nvapi.as_ref().and_then(|n| n.memory_temp());
        hw.cpu_temp = self.cpu_temp.as_ref().and_then(|c| c.read().ok().flatten());
        if let Some(Ok(Some((raw, unit)))) = self.cpu_temp.as_ref().map(|c| c.package_energy()) {
            let now = Instant::now();
            if let Some((prev, t)) = self.energy_prev.replace((raw, now)) {
                let dt = (now - t).as_secs_f64();
                if dt > 0.05 {
                    hw.cpu_power = Some(raw.wrapping_sub(prev) as f64 * unit / dt);
                }
            }
        }
        if let Some(Ok((rad, pump))) = self.fans.as_ref().map(|f| f.read()) {
            hw.fan_radiator = Some(rad);
            hw.fan_pump = Some(pump);
        }
        if let Some(d) = &self.dimms {
            if Instant::now() >= self.ram_next {
                // DIMM temperatures change slowly; spare the SMBus
                self.ram_temp = d.read().ok().flatten();
                self.ram_next = Instant::now() + std::time::Duration::from_secs(5);
            }
        }
        hw.ram_temp = self.ram_temp;
        hw
    }
}
