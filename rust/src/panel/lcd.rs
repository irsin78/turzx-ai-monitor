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
// Storage queries (as the vendor app uses them). The panel runs Linux: the TF card is
// mounted at /tmp/sdcard/mmcblk0p1/ (img/, video/), the built-in flash at /usr/data/.
const CMD_LIST_DIR: u8 = 99;
const CMD_STORAGE: u8 = 100;
// Files and local playback (the panel decodes H.264 itself: raw Annex B stream, portrait
// 480x1920, no B-frames, yuv420p, as the vendor app converts videos).
const CMD_FILE_CREATE: u8 = 38;
const CMD_FILE_CHUNK: u8 = 39;
const CMD_FILE_SMALL: u8 = 40;
const CMD_DELETE: u8 = 42;
const CMD_PLAY_LOCAL: u8 = 110;
const CMD_STOP_LOCAL: u8 = 111;
const CMD_LOCAL_STATE: u8 = 112;
/// Saved settings: brightness, start mode, 0, rotation, sleep delay, keep-on (app.cfg[0..6]).
const CMD_CONFIG: u8 = 125;
pub const CHUNK: usize = 1 << 20;

/// Sizes reported by the panel (CMD_STORAGE), in KB (a 4 GB card reads total 3853376).
#[derive(Debug)]
pub struct Storage {
    pub sd_total: u32,
    pub sd_used: u32,
    pub sd_free: u32,
    pub flash_total: u32,
    pub flash_used: u32,
    pub flash_free: u32,
}

pub struct Lcd {
    out: Endpoint<Bulk, Out>,
    inp: Endpoint<Bulk, In>,
    t0: Instant,
    _iface: Interface,
    pub firmware: String,
    /// USB link speed: frames take ~16 ms at high speed, ~240 ms at full speed
    pub speed: String,
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
        let speed = info.speed().map_or("unknown".to_string(), |s| format!("{s:?}"));
        let device = info.open().wait().context("open TURZX device")?;
        let iface = device.claim_interface(0).wait().context("claim interface 0")?;
        let out = iface.endpoint::<Bulk, Out>(0x01)?;
        let inp = iface.endpoint::<Bulk, In>(0x81)?;
        let mut lcd = Lcd { out, inp, t0: Instant::now(), _iface: iface, firmware: String::new(), speed, tx: Vec::with_capacity(256 * 1024) };
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

    /// A command with up to 492 bytes of arguments at header[8..].
    fn command_with(&mut self, cmd: u8, args: &[u8]) -> Result<Vec<u8>> {
        self.command_wait(cmd, args, Duration::from_secs(2))
    }

    /// `command_with` for commands that touch the TF card: the panel answers after `sync`,
    /// which can take seconds; giving up early would leave its reply for the next command.
    fn command_wait(&mut self, cmd: u8, args: &[u8], wait: Duration) -> Result<Vec<u8>> {
        anyhow::ensure!(args.len() <= 492, "arguments too long");
        let mut h = self.header(cmd);
        h[8..8 + args.len()].copy_from_slice(args);
        self.transfer(Self::encrypt(&h), wait)
    }

    /// Arguments carrying a path: length (BE) at [0..4], the path from [8].
    fn path_args(path: &str) -> Vec<u8> {
        let mut args = vec![0u8; 8 + path.len()];
        args[0..4].copy_from_slice(&(path.len() as u32).to_be_bytes());
        args[8..].copy_from_slice(path.as_bytes());
        args
    }

    /// Write a file on the panel (e.g. /tmp/sdcard/mmcblk0p1/video/x.h264), replacing it.
    pub fn put_file(&mut self, path: &str, data: &[u8]) -> Result<()> {
        anyhow::ensure!(path.len() <= 480, "path too long");
        if data.len() < 100 * 1024 {
            let mut h = self.header(CMD_FILE_SMALL);
            h[8..12].copy_from_slice(&(path.len() as u32).to_be_bytes());
            h[12..16].copy_from_slice(&(data.len() as u32).to_be_bytes());
            h[16..16 + path.len()].copy_from_slice(path.as_bytes());
            let mut buf = Self::encrypt(&h);
            buf.extend_from_slice(data);
            self.transfer(buf, Duration::from_secs(30))?;
            return Ok(());
        }
        self.begin_file(path)?;
        let n = data.len().div_ceil(CHUNK);
        for (i, chunk) in data.chunks(CHUNK).enumerate() {
            self.file_chunk(chunk, i + 1 == n)?;
        }
        Ok(())
    }

    /// Start writing a file (then `file_chunk` with up to `CHUNK` bytes at a time).
    pub fn begin_file(&mut self, path: &str) -> Result<()> {
        let r = self.command_wait(CMD_FILE_CREATE, &Self::path_args(path), Duration::from_secs(30))?;
        anyhow::ensure!(r.get(8) == Some(&200), "panel refused to create {path} (reply {:?})", r.get(8));
        Ok(())
    }

    pub fn file_chunk(&mut self, chunk: &[u8], last: bool) -> Result<()> {
        anyhow::ensure!(chunk.len() <= CHUNK, "chunk too large");
        let mut h = self.header(CMD_FILE_CHUNK);
        h[8..12].copy_from_slice(&(CHUNK as u32).to_be_bytes());
        h[12..16].copy_from_slice(&(chunk.len() as u32).to_be_bytes());
        h[16] = last as u8;
        let mut buf = Self::encrypt(&h);
        buf.extend_from_slice(chunk);
        self.transfer(buf, Duration::from_secs(30))?;
        Ok(())
    }

    /// Delete a file (or directory) on the panel.
    pub fn delete(&mut self, path: &str) -> Result<()> {
        self.command_wait(CMD_DELETE, &Self::path_args(path), Duration::from_secs(30))?;
        Ok(())
    }

    /// Play an H.264 file stored on the panel, on its own (keeps going without the PC).
    pub fn play_local(&mut self, path: &str) -> Result<()> {
        self.command_with(CMD_PLAY_LOCAL, &Self::path_args(path))?;
        Ok(())
    }

    /// Keep the screen on and local playback going after the PC is gone. Without it the panel
    /// turns its backlight off and stops playback 10 s after the USB host disappears (and shows
    /// the frozen frame when the PC comes back). Written as the vendor app's defaults: full
    /// brightness, no start-up playback, no rotation, no sleep delay.
    pub fn keep_on_without_pc(&mut self) -> Result<()> {
        self.command_with(CMD_CONFIG, &[100, 0, 0, 0, 0, 1])?;
        Ok(())
    }

    pub fn stop_local(&mut self) -> Result<()> {
        self.command(CMD_STOP_LOCAL)?;
        Ok(())
    }

    /// The panel's local playback state byte (reply[8]).
    pub fn local_state(&mut self) -> Result<u8> {
        Ok(self.command(CMD_LOCAL_STATE)?.get(8).copied().unwrap_or(0))
    }

    /// TF card and built-in flash sizes (read only).
    pub fn storage(&mut self) -> Result<Storage> {
        let r = self.command(CMD_STORAGE)?;
        anyhow::ensure!(r.len() >= 32, "short storage reply ({} bytes)", r.len());
        let u = |i: usize| u32::from_le_bytes(r[i..i + 4].try_into().unwrap());
        Ok(Storage { sd_total: u(8), sd_used: u(12), sd_free: u(16), flash_total: u(20), flash_used: u(24), flash_free: u(28) })
    }

    /// Listing of a directory on the panel: the vendor app asks 20 times and joins
    /// the replies ("dir:a/b/file:c/"). A missing directory is created ("nodir-create"), so
    /// this is not strictly read-only. Returned raw.
    pub fn list_dir(&mut self, path: &str) -> Result<Vec<Vec<u8>>> {
        let args = Self::path_args(path);
        (0..20).map(|_| self.command_with(CMD_LIST_DIR, &args)).collect()
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
    /// 90 degrees CCW, so the image is pre-rotated 90 degrees CW (CCW with `rotation = 180`).
    pub fn show_landscape(&mut self, rgb: &[u8]) -> Result<()> {
        let mut portrait = rotate_cw(rgb, HEIGHT, WIDTH);
        if crate::config::get().rotation == 180 {
            // CW then 180 = CCW: reverse the pixel order
            let n = portrait.len() / 3;
            for i in 0..n / 2 {
                for c in 0..3 {
                    portrait.swap(i * 3 + c, (n - 1 - i) * 3 + c);
                }
            }
        }
        let jpeg = encode_jpeg(&portrait, WIDTH, HEIGHT)?;
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
