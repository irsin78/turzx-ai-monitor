//! CPU package temperature, DDR5 DIMM temperatures and board fan RPMs via PawnIO.
//!
//! Needs the PawnIO driver (installed here by FanControl) and administrator rights.
//! The signed PawnIO.Modules blobs (LGPL-2.1) are embedded. Register usage follows
//! LibreHardwareMonitor (MPL-2.0) and RAMSPDToolkit (MPL-2.0).
//! Only reads, apart from the Super I/O index/bank selection needed to read.

use std::ffi::{c_char, c_void, CString};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use libloading::Library;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

const LIB: &str = r"C:\Program Files\PawnIO\PawnIOLib.dll";
const INTEL_MSR: &[u8] = include_bytes!("../../pawnio_modules/IntelMSR.bin");
const LPC_IO: &[u8] = include_bytes!("../../pawnio_modules/LpcIO.bin");
const SMBUS_I801: &[u8] = include_bytes!("../../pawnio_modules/SmbusI801.bin");

type OpenFn = unsafe extern "system" fn(*mut *mut c_void) -> i32;
type LoadFn = unsafe extern "system" fn(*mut c_void, *const u8, usize) -> i32;
type ExecFn = unsafe extern "system" fn(*mut c_void, *const c_char, *const u64, usize, *mut u64, usize, *mut usize) -> i32;

pub fn is_admin() -> bool {
    unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() }
}

struct Module {
    lib: Arc<Library>,
    handle: *mut c_void,
}

// The handle is only used from the hardware thread.
unsafe impl Send for Module {}

impl Module {
    fn new(lib: &Arc<Library>, blob: &[u8], name: &str) -> Result<Self> {
        let mut handle = std::ptr::null_mut();
        unsafe {
            let open: libloading::Symbol<OpenFn> = lib.get(b"pawnio_open")?;
            let hr = open(&mut handle);
            if hr < 0 {
                let who = if is_admin() { "admin" } else { "NOT admin" };
                bail!("pawnio_open failed 0x{:08X} ({who})", hr as u32);
            }
            let load: libloading::Symbol<LoadFn> = lib.get(b"pawnio_load")?;
            let hr = load(handle, blob.as_ptr(), blob.len());
            if hr < 0 {
                bail!("pawnio_load {name} failed 0x{:08X}", hr as u32);
            }
        }
        Ok(Module { lib: lib.clone(), handle })
    }

    fn call(&self, func: &str, args: &[u64], out: usize) -> Result<Vec<u64>> {
        let name = CString::new(func)?;
        let mut o = vec![0u64; out.max(1)];
        let mut n = 0usize;
        let hr = unsafe {
            let exec: libloading::Symbol<ExecFn> = self.lib.get(b"pawnio_execute")?;
            exec(self.handle, name.as_ptr(), args.as_ptr(), args.len(), o.as_mut_ptr(), out, &mut n)
        };
        if hr < 0 {
            bail!("{func} failed: 0x{:08X}", hr as u32);
        }
        o.truncate(out);
        Ok(o)
    }
}

/// Mutexes shared by hardware tools (LHM, HWiNFO, FanControl) to serialize bus access.
struct GlobalMutex(HANDLE);

impl GlobalMutex {
    fn lock(name: &str) -> Result<Self> {
        let wide: Vec<u16> = format!("Global\\{name}").encode_utf16().chain([0]).collect();
        unsafe {
            let h = CreateMutexW(None, false, PCWSTR(wide.as_ptr()))?;
            let r = WaitForSingleObject(h, 500);
            if r != WAIT_OBJECT_0 && r != WAIT_ABANDONED {
                let _ = CloseHandle(h);
                bail!("timed out waiting for {name}");
            }
            Ok(GlobalMutex(h))
        }
    }
}

impl Drop for GlobalMutex {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

pub fn library() -> Result<Arc<Library>> {
    Ok(Arc::new(unsafe { Library::new(LIB) }.context("PawnIOLib.dll not installed")?))
}

pub struct IntelCpuTemp {
    m: Module,
    tjmax: u64,
    energy_unit: Option<f64>, // joules per RAPL count
}

impl IntelCpuTemp {
    const TEMPERATURE_TARGET: u64 = 0x1A2;
    const PACKAGE_THERM_STATUS: u64 = 0x1B1;
    const RAPL_POWER_UNIT: u64 = 0x606;
    const PKG_ENERGY_STATUS: u64 = 0x611;

    pub fn new(lib: &Arc<Library>) -> Result<Self> {
        let m = Module::new(lib, INTEL_MSR, "IntelMSR")?;
        let tjmax = (m.call("ioctl_read_msr", &[Self::TEMPERATURE_TARGET], 1)?[0] >> 16) & 0xFF;
        let energy_unit = m
            .call("ioctl_read_msr", &[Self::RAPL_POWER_UNIT], 1)
            .map(|v| 0.5f64.powi(((v[0] >> 8) & 0x1F) as i32))
            .map_err(|e| log::warn!("RAPL unavailable: {e}"))
            .ok();
        Ok(IntelCpuTemp { m, tjmax, energy_unit })
    }

    pub fn read(&self) -> Result<Option<f64>> {
        let status = self.m.call("ioctl_read_msr", &[Self::PACKAGE_THERM_STATUS], 1)?[0];
        if status & (1 << 31) == 0 {
            return Ok(None); // reading not valid
        }
        Ok(Some((self.tjmax - ((status >> 16) & 0x7F)) as f64))
    }

    /// Package energy counter (32-bit, wraps) and its unit in joules.
    pub fn package_energy(&self) -> Result<Option<(u32, f64)>> {
        let Some(unit) = self.energy_unit else { return Ok(None) };
        let raw = self.m.call("ioctl_read_msr", &[Self::PKG_ENERGY_STATUS], 1)?[0] as u32;
        Ok(Some((raw, unit)))
    }
}

/// Nuvoton NCT6796D-R / NCT5585D hardware monitor (chip ID 0xD4xx), at Super I/O port 0x2E on
/// this board (0x4E holds an unrelated chip, ID 0xA3A3), so both slots are probed.
pub struct NuvotonFans {
    m: Module,
    base: u64,
}

impl NuvotonFans {
    const MUTEX: &'static str = "Access_ISABUS.HTP.Method";
    const SLOTS: [(u64, u64); 2] = [(0, 0x2E), (1, 0x4E)];
    // 13-bit fan count registers; RPM = 1.35M / count. Fan #2 = header CPU1, Fan #7 = AIO_PUMP
    const RADIATOR: u64 = 0x4B2;
    const PUMP: u64 = 0x4CC;

    pub fn new(lib: &Arc<Library>) -> Result<Self> {
        let m = Module::new(lib, LPC_IO, "LpcIO")?;
        let _g = GlobalMutex::lock(Self::MUTEX)?;
        let mut seen = Vec::new();
        for (slot, port) in Self::SLOTS {
            m.call("ioctl_select_slot", &[slot], 0)?;
            m.call("ioctl_pio_outb", &[port, 0x87], 0)?; // enter config mode
            m.call("ioctl_pio_outb", &[port, 0x87], 0)?;
            let found = (|| -> Result<Option<u64>> {
                let chip = m.call("ioctl_superio_inw", &[0x20], 1)?[0];
                seen.push(format!("0x{port:02X}:0x{chip:04X}"));
                if chip >> 8 != 0xD4 {
                    return Ok(None);
                }
                m.call("ioctl_find_bars", &[], 0)?;
                m.call("ioctl_superio_outb", &[0x07, 0x0B], 0)?; // hardware monitor LDN
                Ok(Some(m.call("ioctl_superio_inw", &[0x60], 1)?[0] & 0xFFF8))
            })();
            m.call("ioctl_pio_outb", &[port, 0xAA], 0)?; // exit config mode
            if let Some(base) = found? {
                drop(_g);
                return Ok(NuvotonFans { m, base });
            }
        }
        bail!("Nuvoton Super I/O not found ({})", seen.join(", "))
    }

    fn reg(&self, reg: u64) -> Result<u64> {
        let (addr, data) = (self.base + 5, self.base + 6);
        self.m.call("ioctl_pio_outb", &[addr, 0x4E], 0)?; // bank select
        self.m.call("ioctl_pio_outb", &[data, reg >> 8], 0)?;
        self.m.call("ioctl_pio_outb", &[addr, reg & 0xFF], 0)?;
        Ok(self.m.call("ioctl_pio_inb", &[data], 1)?[0])
    }

    fn rpm(&self, reg: u64) -> Result<f64> {
        let count = (self.reg(reg)? << 5) | (self.reg(reg + 1)? & 0x1F);
        Ok(if count > 0 && count < 0x1FFF { 1.35e6 / count as f64 } else { 0.0 })
    }

    /// (radiator, pump) RPM
    pub fn read(&self) -> Result<(f64, f64)> {
        let _g = GlobalMutex::lock(Self::MUTEX)?;
        Ok((self.rpm(Self::RADIATOR)?, self.rpm(Self::PUMP)?))
    }
}

/// DDR5 SPD hub (SPD5118) thermal sensor, read-only: MR49/50, 0.0625 C/LSB.
pub struct Ddr5Temps {
    m: Module,
    addrs: Vec<u64>,
}

impl Ddr5Temps {
    const MUTEX: &'static str = "Access_SMBUS.HTP.Method";
    const READ: u64 = 1;
    const BYTE_DATA: u64 = 2;
    const WORD_DATA: u64 = 3;

    pub fn new(lib: &Arc<Library>) -> Result<Self> {
        let m = Module::new(lib, SMBUS_I801, "SmbusI801")?;
        let mut addrs = Vec::new();
        {
            let _g = GlobalMutex::lock(Self::MUTEX)?;
            for addr in 0x50..0x58 {
                // device type MR0/MR1 = 0x51 0x18 identifies an SPD5118 hub
                let ty = |reg| m.call("ioctl_smbus_xfer", &[addr, Self::READ, reg, Self::BYTE_DATA], 1);
                if let (Ok(a), Ok(b)) = (ty(0), ty(1)) {
                    if (a[0] & 0xFF, b[0] & 0xFF) == (0x51, 0x18) {
                        addrs.push(addr);
                    }
                }
            }
        }
        if addrs.is_empty() {
            bail!("no DDR5 SPD hub found");
        }
        Ok(Ddr5Temps { m, addrs })
    }

    /// Hottest DIMM
    pub fn read(&self) -> Result<Option<f64>> {
        let _g = GlobalMutex::lock(Self::MUTEX)?;
        let mut hottest: Option<f64> = None;
        for &addr in &self.addrs {
            let raw = self.m.call("ioctl_smbus_xfer", &[addr, Self::READ, 0x31, Self::WORD_DATA], 1)?[0] & 0x1FFF;
            let t = (raw & 0xFFF) as f64 * 0.0625 - if raw & 0x1000 != 0 { 256.0 } else { 0.0 };
            hottest = Some(hottest.map_or(t, |h: f64| h.max(t)));
        }
        Ok(hottest)
    }
}
