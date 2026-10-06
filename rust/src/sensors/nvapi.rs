//! GPU memory (VRAM) temperature through NvAPI, which NVML does not report.

use std::ffi::c_void;

use anyhow::{bail, Result};
use libloading::Library;

/// GPU memory junction temperature via NvAPI's undocumented GPU_GetThermalSensors.
/// Interface IDs, struct layout and the RTX 50 index follow LibreHardwareMonitor (MPL-2.0).
pub struct NvApiThermal {
    _lib: Library,
    get: unsafe extern "C" fn(*mut c_void, *mut ThermalSensors) -> i32,
    gpu: *mut c_void,
    mask: u32,
}

#[repr(C)]
struct ThermalSensors {
    version: u32,
    mask: u32,
    reserved: [i32; 8],
    temps: [i32; 32],
}

unsafe impl Send for NvApiThermal {}

impl NvApiThermal {
    const MEMORY_INDEX: usize = 2; // RTX 50xx (RTX 40xx: 7, older: 9)

    pub fn new() -> Result<Self> {
        unsafe {
            let lib = Library::new("nvapi64.dll")?;
            let query: libloading::Symbol<unsafe extern "C" fn(u32) -> *mut c_void> = lib.get(b"nvapi_QueryInterface")?;
            let init: unsafe extern "C" fn() -> i32 = std::mem::transmute(query(0x0150E828));
            let enum_gpus: unsafe extern "C" fn(*mut *mut c_void, *mut i32) -> i32 = std::mem::transmute(query(0xE5AC921F));
            let get: unsafe extern "C" fn(*mut c_void, *mut ThermalSensors) -> i32 = std::mem::transmute(query(0x65FE3AAD));
            if init() != 0 {
                bail!("NvAPI_Initialize failed");
            }
            let mut handles = [std::ptr::null_mut(); 64];
            let mut count = 0;
            enum_gpus(handles.as_mut_ptr(), &mut count);
            if count < 1 {
                bail!("no NVIDIA GPU");
            }
            let mut me = NvApiThermal { _lib: lib, get, gpu: handles[0], mask: 0 };
            // The mask must cover exactly the supported sensors: probe for the highest valid bit
            for bit in 0..32 {
                if me.query(1 << bit).is_none() {
                    break;
                }
                me.mask = (2u64 << bit).wrapping_sub(1) as u32;
            }
            Ok(me)
        }
    }

    fn query(&self, mask: u32) -> Option<ThermalSensors> {
        let mut t = ThermalSensors {
            version: std::mem::size_of::<ThermalSensors>() as u32 | (2 << 16),
            mask,
            reserved: [0; 8],
            temps: [0; 32],
        };
        (unsafe { (self.get)(self.gpu, &mut t) } == 0).then_some(t)
    }

    pub fn memory_temp(&self) -> Option<f64> {
        if self.mask == 0 {
            return None;
        }
        self.query(self.mask).map(|t| t.temps[Self::MEMORY_INDEX] as f64 / 256.0)
    }
}
