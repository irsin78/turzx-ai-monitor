//! SSD / HDD temperatures through the Windows storage stack (IOCTL_STORAGE_QUERY_PROPERTY,
//! StorageDeviceTemperatureProperty): the drive's composite temperature, no SMART tool needed.
//!
//! Which drives: the ones holding the drive letters in the settings (`[disks] show`), or else
//! the first two internal drives (USB and virtual disks are skipped). Each is labeled with its
//! model, shortened ("Samsung SSD 990 PRO 1TB" -> "990 PRO", "CT2000T710SSD8" -> "T710"), or the
//! name in `[disks] labels`, or "Disk N" (the Windows disk number) when the model is unknown.

use std::collections::BTreeMap;

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    BusTypeFileBackedVirtual, BusTypeUsb, CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, StorageDeviceTemperatureProperty, IOCTL_STORAGE_GET_DEVICE_NUMBER,
    IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_DEVICE_DESCRIPTOR, STORAGE_DEVICE_NUMBER, STORAGE_PROPERTY_QUERY,
    STORAGE_TEMPERATURE_DATA_DESCRIPTOR,
};
use windows::Win32::System::IO::DeviceIoControl;

use crate::config;

pub const MAX: usize = 2;

/// A drive to watch: its physical disk number and label ("990 PRO").
pub struct Disk {
    number: u32,
    pub label: String,
}

/// Open a device for queries only (no read/write access needed for these IOCTLs).
fn open(path: &str) -> Option<HANDLE> {
    unsafe {
        CreateFileW(&HSTRING::from(path), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None).ok()
    }
}

fn ioctl<T: Default>(h: HANDLE, code: u32, input: Option<&STORAGE_PROPERTY_QUERY>) -> Option<T> {
    let mut out = T::default();
    let mut got = 0u32;
    let (inp, inlen) = match input {
        Some(q) => (Some(q as *const _ as *const core::ffi::c_void), std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32),
        None => (None, 0),
    };
    unsafe {
        DeviceIoControl(h, code, inp, inlen, Some(&mut out as *mut T as *mut _), std::mem::size_of::<T>() as u32, Some(&mut got), None).ok()?;
    }
    Some(out)
}

fn query<T: Default>(h: HANDLE, property: windows::Win32::System::Ioctl::STORAGE_PROPERTY_ID) -> Option<T> {
    let q = STORAGE_PROPERTY_QUERY { PropertyId: property, QueryType: PropertyStandardQuery, ..Default::default() };
    ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, Some(&q))
}

/// Internal (not USB or virtual) physical disks and their drive letters.
fn drives() -> BTreeMap<u32, (bool, Vec<char>)> {
    let mut map: BTreeMap<u32, (bool, Vec<char>)> = BTreeMap::new();
    for letter in 'C'..='Z' {
        let Some(h) = open(&format!(r"\\.\{letter}:")) else { continue };
        if let Some(n) = ioctl::<STORAGE_DEVICE_NUMBER>(h, IOCTL_STORAGE_GET_DEVICE_NUMBER, None) {
            map.entry(n.DeviceNumber).or_insert_with(|| (internal(n.DeviceNumber), Vec::new())).1.push(letter);
        }
        unsafe {
            let _ = CloseHandle(h);
        }
    }
    map
}

fn internal(number: u32) -> bool {
    let Some(h) = open(&format!(r"\\.\PhysicalDrive{number}")) else { return false };
    let d = query::<STORAGE_DEVICE_DESCRIPTOR>(h, StorageDeviceProperty);
    unsafe {
        let _ = CloseHandle(h);
    }
    d.is_some_and(|d| d.BusType != BusTypeUsb && d.BusType != BusTypeFileBackedVirtual && !d.RemovableMedia)
}

/// The drive's product id ("Samsung SSD 990 PRO 1TB").
fn model(number: u32) -> Option<String> {
    let h = open(&format!(r"\\.\PhysicalDrive{number}"))?;
    let q = STORAGE_PROPERTY_QUERY { PropertyId: StorageDeviceProperty, QueryType: PropertyStandardQuery, ..Default::default() };
    let mut buf = vec![0u8; 1024]; // the descriptor plus its strings
    let mut got = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&q as *const _ as *const _),
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(buf.as_mut_ptr().cast()),
            buf.len() as u32,
            Some(&mut got),
            None,
        )
        .is_ok()
    };
    unsafe {
        let _ = CloseHandle(h);
    }
    if !ok || (got as usize) < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return None;
    }
    let d: STORAGE_DEVICE_DESCRIPTOR = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast()) };
    let at = d.ProductIdOffset as usize;
    if at == 0 || at >= got as usize {
        return None;
    }
    let end = buf[at..got as usize].iter().position(|&b| b == 0).map_or(got as usize, |n| at + n);
    Some(String::from_utf8_lossy(&buf[at..end]).trim().to_string())
}

/// The model part of a product id: no vendor, "SSD", capacity or part-number wrapper
/// ("Samsung SSD 990 PRO 1TB" -> "990 PRO", "CT2000T710SSD8" -> "T710").
fn short_model(id: &str) -> String {
    const VENDORS: [&str; 17] = [
        "samsung", "crucial", "wdc", "wd", "western", "digital", "kingston", "sk", "hynix", "seagate", "sabrent", "adata", "lexar",
        "micron", "intel", "toshiba", "kioxia",
    ];
    let capacity = |t: &str| {
        let l = t.to_ascii_lowercase();
        let digits = l.trim_end_matches("gb").trim_end_matches("tb").trim_end_matches('g').trim_end_matches('t');
        digits != l && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
    };
    let words: Vec<String> = id
        .split_whitespace()
        .filter(|w| !VENDORS.contains(&w.to_ascii_lowercase().as_str()) && !w.eq_ignore_ascii_case("ssd") && !capacity(w))
        .map(|w| {
            // Crucial part numbers: CT2000T710SSD8 -> T710
            let up = w.to_ascii_uppercase();
            match (up.strip_prefix("CT"), up.find("SSD")) {
                (Some(rest), Some(i)) if i > 2 => rest[..i - 2].trim_start_matches(|c: char| c.is_ascii_digit()).to_string(),
                _ => w.to_string(),
            }
        })
        .filter(|w| !w.is_empty())
        .collect();
    words.join(" ")
}

#[cfg(test)]
mod tests {
    #[test]
    fn short_models() {
        assert_eq!(super::short_model("Samsung SSD 990 PRO 1TB"), "990 PRO");
        assert_eq!(super::short_model("CT2000T710SSD8"), "T710");
        assert_eq!(super::short_model("WD_BLACK SN850X 2000GB"), "WD_BLACK SN850X");
    }
}

/// The drives to show (see the module comment).
pub fn pick() -> Vec<Disk> {
    let map = drives();
    let wanted: Vec<char> = config::get().disks.show.iter().filter_map(|s| s.trim().chars().next()).map(|c| c.to_ascii_uppercase()).collect();
    let mut out: Vec<Disk> = Vec::new();
    let mut add = |number: u32| {
        if out.len() < MAX && !out.iter().any(|d| d.number == number) {
            let custom = config::get().disks.labels.get(out.len()).filter(|l| !l.trim().is_empty()).cloned();
            let label = custom.or_else(|| model(number).map(|m| short_model(&m))).filter(|l| !l.is_empty()).unwrap_or_else(|| format!("Disk {number}"));
            out.push(Disk { number, label });
        }
    };
    if wanted.is_empty() {
        for (n, (inside, _)) in &map {
            if *inside {
                add(*n);
            }
        }
    } else {
        for w in wanted {
            if let Some((n, _)) = map.iter().find(|(_, (_, letters))| letters.contains(&w)) {
                add(*n);
            }
        }
    }
    out
}

impl Disk {
    /// Composite temperature in °C.
    pub fn temperature(&self) -> Option<f64> {
        let h = open(&format!(r"\\.\PhysicalDrive{}", self.number))?;
        let t = query::<STORAGE_TEMPERATURE_DATA_DESCRIPTOR>(h, StorageDeviceTemperatureProperty);
        unsafe {
            let _ = CloseHandle(h);
        }
        let t = t?;
        (t.InfoCount > 0).then(|| t.TemperatureInfo[0].Temperature as f64).filter(|&c| c > 0.0 && c < 150.0)
    }
}
