//! TURZX 8.8" USB LCD (VID 0x1CBE / PID 0x0088, 480x1920 portrait).
//!
//! Protocol (reverse-engineered from TURZX.exe):
//! - 500-byte command header: [0]=cmd, [2]=0x1A, [3]=0x6D, [4..8]=ms timestamp (LE), [8..]=payload
//! - header is DES-CBC encrypted (key = IV = "slv3tuzx", PKCS7) -> 504 bytes, placed in a
//!   512-byte block with [510]=0xA1, [511]=0x1A
//! - bulk OUT 0x01, IN 0x81; every 512-byte reply is followed by a zero-length packet
//! - image: cmd 101 (JPEG), header[8..12] = length (BE), file bytes appended to the block

use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use cipher::{block_padding::Pkcs7, BlockModeEncrypt, KeyIvInit};
use jpeg_encoder::{ColorType, Encoder, SamplingFactor};
use nusb::transfer::{Buffer, Bulk, In, Out};
use nusb::{Endpoint, Interface, MaybeFuture};

pub const VID: u16 = 0x1CBE;
pub const PID: u16 = 0x0088;
pub const WIDTH: usize = 480; // portrait framebuffer
pub const HEIGHT: usize = 1920;

const KEY: &[u8; 8] = b"slv3tuzx";
const CMD_GET_VERSION: u8 = 10;
const CMD_JPEG: u8 = 101;

pub struct Lcd {
    out: Endpoint<Bulk, Out>,
    inp: Endpoint<Bulk, In>,
    t0: Instant,
    _iface: Interface,
    pub firmware: String,
    tx: Vec<u8>, // reused for every frame
}

/// Why the panel could not be opened (the not-found case is normal while it is unplugged).
#[derive(Debug)]
pub struct NotFound;

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("TURZX device not found")
    }
}

impl std::error::Error for NotFound {}

impl Lcd {
    /// Open the panel and check that it answers. A panel that was just re-plugged or knocked
    /// loose can come back with stalled endpoints: clear the halt and ask once more.
    pub fn open() -> Result<Self> {
        let info = nusb::list_devices()
            .wait()?
            .find(|d| d.vendor_id() == VID && d.product_id() == PID)
            .ok_or(NotFound)?;
        let device = info.open().wait().context("open TURZX device")?;
        let iface = device.claim_interface(0).wait().context("claim interface 0")?;
        let out = iface.endpoint::<Bulk, Out>(0x01)?;
        let inp = iface.endpoint::<Bulk, In>(0x81)?;
        let mut lcd = Lcd { out, inp, t0: Instant::now(), _iface: iface, firmware: String::new(), tx: Vec::with_capacity(256 * 1024) };
        lcd.drain();
        lcd.firmware = match lcd.version() {
            Ok(v) => v,
            Err(first) => {
                lcd.clear_halts();
                lcd.drain();
                lcd.version().with_context(|| format!("no answer after clearing a halt (first try: {first:#})"))?
            }
        };
        Ok(lcd)
    }

    fn clear_halts(&mut self) {
        let _ = self.out.clear_halt().wait();
        let _ = self.inp.clear_halt().wait();
    }

    /// Discard stale replies left in the IN pipe.
    fn drain(&mut self) {
        loop {
            let c = self.inp.transfer_blocking(Buffer::new(512), Duration::from_millis(100));
            if c.status.is_err() {
                return;
            }
        }
    }

    fn header(&self, cmd: u8) -> [u8; 500] {
        let mut h = [0u8; 500];
        h[0] = cmd;
        h[2] = 0x1A;
        h[3] = 0x6D;
        let ms = self.t0.elapsed().as_millis() as u32;
        h[4..8].copy_from_slice(&ms.to_le_bytes());
        h
    }

    fn encrypt(h: &[u8; 500]) -> Vec<u8> {
        let enc = cbc::Encryptor::<des::Des>::new_from_slices(KEY, KEY)
            .expect("8-byte key and IV")
            .encrypt_padded_vec::<Pkcs7>(h);
        let mut block = vec![0u8; 512];
        block[..enc.len()].copy_from_slice(&enc);
        block[510] = 0xA1;
        block[511] = 0x1A;
        block
    }

    fn transfer(&mut self, data: Vec<u8>, timeout: Duration) -> Result<Vec<u8>> {
        let c = self.out.transfer_blocking(Buffer::from(data), timeout);
        c.status.context("bulk OUT")?;
        self.tx = c.buffer.into_vec(); // keep the allocation for the next frame
        // Each reply is followed by a zero-length packet; skip ZLPs.
        for _ in 0..4 {
            let c = self.inp.transfer_blocking(Buffer::new(512), timeout);
            c.status.context("bulk IN")?;
            if c.actual_len > 0 {
                let mut v = c.buffer.into_vec();
                v.truncate(c.actual_len);
                return Ok(v);
            }
        }
        bail!("no reply from LCD")
    }

    pub fn command(&mut self, cmd: u8) -> Result<Vec<u8>> {
        let block = Self::encrypt(&self.header(cmd));
        self.transfer(block, Duration::from_secs(2))
    }

    fn version(&mut self) -> Result<String> {
        let r = self.command(CMD_GET_VERSION)?;
        let s = &r[8..40.min(r.len())];
        Ok(String::from_utf8_lossy(s).trim_end_matches('\0').to_string())
    }

    pub fn send_jpeg(&mut self, jpeg: &[u8]) -> Result<Vec<u8>> {
        let mut h = self.header(CMD_JPEG);
        h[8..12].copy_from_slice(&(jpeg.len() as u32).to_be_bytes());
        let mut data = std::mem::take(&mut self.tx);
        data.clear();
        data.extend_from_slice(&Self::encrypt(&h));
        data.extend_from_slice(jpeg);
        self.transfer(data, Duration::from_secs(5))
    }

    /// Show a landscape 1920x480 RGB image (row-major, 3 bytes/px).
    ///
    /// With the monitor lying landscape the panel shows the portrait framebuffer rotated
    /// 90 degrees CCW, so the image is pre-rotated 90 degrees CW.
    pub fn show_landscape(&mut self, rgb: &[u8]) -> Result<()> {
        let jpeg = encode_jpeg(&rotate_cw(rgb, HEIGHT, WIDTH), WIDTH, HEIGHT)?;
        self.send_jpeg(&jpeg)?;
        Ok(())
    }
}

/// Rotate a w x h RGB image 90 degrees clockwise (result is h x w).
pub fn rotate_cw(rgb: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; rgb.len()];
    for y in 0..h {
        for x in 0..w {
            // old (x, y) -> new (h - 1 - y, x) in an h-wide image
            let src = (y * w + x) * 3;
            let dst = (x * h + (h - 1 - y)) * 3;
            out[dst..dst + 3].copy_from_slice(&rgb[src..src + 3]);
        }
    }
    out
}

/// JPEG quality 95 with 4:4:4 chroma: 4:2:0 smears colored text into "dead pixel" specks.
pub fn encode_jpeg(rgb: &[u8], w: usize, h: usize) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(256 * 1024);
    let mut enc = Encoder::new(&mut buf, 95);
    enc.set_sampling_factor(SamplingFactor::F_1_1);
    enc.encode(rgb, w as u16, h as u16, ColorType::Rgb)?;
    Ok(buf)
}
