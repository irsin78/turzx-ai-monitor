//! Baseline JPEG encoder for the panel that re-encodes only the rows that changed.
//!
//! The panel takes whole JPEG frames (cmd 101), but between dashboard redraws only the mascots
//! move. With a restart interval of one MCU row (DRI), every 8-pixel band of the portrait
//! frame is entropy-coded on its own with DC prediction reset, so the bytes of unchanged bands
//! can be reused as they are and only the dirty bands are encoded again.
//!
//! Fixed for this panel: 480x1920 portrait (the landscape canvas rotated 90 degrees clockwise,
//! read straight from the canvas, no intermediate buffers), YCbCr 4:4:4 (4:2:0 smears colored
//! text), quality 95, standard Huffman tables (ITU T.81 Annex K).

use std::ops::Range;

use tiny_skia::Pixmap;

pub const WIDTH: usize = 480; // portrait
pub const HEIGHT: usize = 1920;
pub const BANDS: usize = HEIGHT / 8; // MCU rows; band b = landscape columns 8b..8b+8
const MCUS: usize = WIDTH / 8; // per band

const QUALITY: u32 = 95;

#[rustfmt::skip]
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[rustfmt::skip]
const LUMA_Q: [u32; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80, 62,
    18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];

#[rustfmt::skip]
const CHROMA_Q: [u32; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99, 47, 66, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

const DC_LUMA_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_CHROMA_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_VALUES: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_LUMA_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7D];
const AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];

#[rustfmt::skip]
const AC_LUMA_VALUES: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08,
    0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
    0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6,
    0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
    0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA,
];

#[rustfmt::skip]
const AC_CHROMA_VALUES: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91,
    0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0, 0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26,
    0x27, 0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58,
    0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4,
    0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA,
    0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA,
];

/// (code, length) by symbol.
struct Huff([(u16, u8); 256]);

impl Huff {
    fn new(bits: &[u8; 16], values: &[u8]) -> Self {
        let mut t = [(0u16, 0u8); 256];
        let (mut code, mut k) = (0u16, 0);
        for (len, &n) in bits.iter().enumerate() {
            for _ in 0..n {
                t[values[k] as usize] = (code, len as u8 + 1);
                code += 1;
                k += 1;
            }
            code <<= 1;
        }
        Huff(t)
    }
}

/// Scaled quantization table (IJG quality formula) in natural order.
fn quant(base: &[u32; 64]) -> [u8; 64] {
    let scale = if QUALITY < 50 { 5000 / QUALITY } else { 200 - QUALITY * 2 };
    base.map(|q| ((q * scale + 50) / 100).clamp(1, 255) as u8)
}

/// Reciprocal divisors for the AAN float DCT output (folds in its scale factors).
fn divisors(q: &[u8; 64]) -> [f32; 64] {
    const AAN: [f32; 8] = [1.0, 1.387_039_8, 1.306_563, 1.175_875_6, 1.0, 0.785_694_96, 0.541_196_1, 0.275_899_38];
    std::array::from_fn(|i| 1.0 / (q[i] as f32 * AAN[i / 8] * AAN[i % 8] * 8.0))
}

/// AAN forward DCT in place (IJG jfdctflt).
fn fdct(d: &mut [f32; 64]) {
    for pass in 0..2 {
        for i in 0..8 {
            // rows on the first pass, columns on the second
            let (base, step) = if pass == 0 { (i * 8, 1) } else { (i, 8) };
            let at = |k: usize| base + k * step;
            let (t0, t7) = (d[at(0)] + d[at(7)], d[at(0)] - d[at(7)]);
            let (t1, t6) = (d[at(1)] + d[at(6)], d[at(1)] - d[at(6)]);
            let (t2, t5) = (d[at(2)] + d[at(5)], d[at(2)] - d[at(5)]);
            let (t3, t4) = (d[at(3)] + d[at(4)], d[at(3)] - d[at(4)]);
            let (t10, t13, t11, t12) = (t0 + t3, t0 - t3, t1 + t2, t1 - t2);
            d[at(0)] = t10 + t11;
            d[at(4)] = t10 - t11;
            let z1 = (t12 + t13) * 0.707_106_77;
            d[at(2)] = t13 + z1;
            d[at(6)] = t13 - z1;
            let (t10, t11, t12) = (t4 + t5, t5 + t6, t6 + t7);
            let z5 = (t10 - t12) * 0.382_683_43;
            let z2 = 0.541_196_1 * t10 + z5;
            let z4 = 1.306_563 * t12 + z5;
            let z3 = t11 * 0.707_106_77;
            let (z11, z13) = (t7 + z3, t7 - z3);
            d[at(5)] = z13 + z2;
            d[at(3)] = z13 - z2;
            d[at(1)] = z11 + z4;
            d[at(7)] = z11 - z4;
        }
    }
}

/// Bit writer with JPEG byte stuffing.
struct Bits<'a> {
    out: &'a mut Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits<'_> {
    #[inline]
    fn put(&mut self, code: u32, len: u32) {
        self.acc = (self.acc << len) | (code & ((1 << len) - 1));
        self.n += len;
        while self.n >= 8 {
            self.n -= 8;
            let b = (self.acc >> self.n) as u8;
            self.out.push(b);
            if b == 0xFF {
                self.out.push(0);
            }
        }
        self.acc &= (1 << self.n) - 1;
    }

    /// Pad to a byte boundary with 1-bits (before a restart marker / EOI).
    fn flush(&mut self) {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
    }
}

/// Magnitude category and the extra bits of a coefficient.
#[inline]
fn category(v: i32) -> (u32, u32) {
    let a = v.unsigned_abs();
    let cat = 32 - a.leading_zeros();
    let bits = if v < 0 { (v - 1) as u32 } else { v as u32 };
    (cat, bits & ((1 << cat) - 1))
}

pub struct RowJpeg {
    header: Vec<u8>,
    bands: Vec<Vec<u8>>, // entropy-coded bytes per band, padded and stuffed
    hashes: Vec<u64>,     // pixels each band was encoded from; unchanged bands are skipped
    frame: Vec<u8>,
    div: [[f32; 64]; 2], // luma, chroma
    dc: [Huff; 2],
    ac: [Huff; 2],
}

impl RowJpeg {
    pub fn new() -> Self {
        let (ql, qc) = (quant(&LUMA_Q), quant(&CHROMA_Q));
        let mut h = vec![0xFF, 0xD8]; // SOI
        let mut seg = |marker: u8, body: &[u8]| {
            h.extend_from_slice(&[0xFF, marker]);
            h.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
            h.extend_from_slice(body);
        };
        seg(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        for (id, q) in [(0u8, &ql), (1, &qc)] {
            let mut b = vec![id];
            b.extend(ZIGZAG.iter().map(|&n| q[n]));
            seg(0xDB, &b);
        }
        let mut sof = vec![8];
        sof.extend_from_slice(&(HEIGHT as u16).to_be_bytes());
        sof.extend_from_slice(&(WIDTH as u16).to_be_bytes());
        sof.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1]);
        seg(0xC0, &sof);
        for (class_id, bits, vals) in [
            (0x00u8, &DC_LUMA_BITS, &DC_VALUES[..]),
            (0x10, &AC_LUMA_BITS, &AC_LUMA_VALUES[..]),
            (0x01, &DC_CHROMA_BITS, &DC_VALUES[..]),
            (0x11, &AC_CHROMA_BITS, &AC_CHROMA_VALUES[..]),
        ] {
            let mut b = vec![class_id];
            b.extend_from_slice(bits);
            b.extend_from_slice(vals);
            seg(0xC4, &b);
        }
        seg(0xDD, &(MCUS as u16).to_be_bytes()); // restart every band
        seg(0xDA, &[3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);
        RowJpeg {
            header: h,
            bands: vec![Vec::new(); BANDS],
            hashes: vec![0; BANDS],
            frame: Vec::with_capacity(256 * 1024),
            div: [divisors(&ql), divisors(&qc)],
            dc: [Huff::new(&DC_LUMA_BITS, &DC_VALUES), Huff::new(&DC_CHROMA_BITS, &DC_VALUES)],
            ac: [Huff::new(&AC_LUMA_BITS, &AC_LUMA_VALUES), Huff::new(&AC_CHROMA_BITS, &AC_CHROMA_VALUES)],
        }
    }

    /// Re-encode bands `bands` from the landscape canvas (1920x480, opaque).
    pub fn encode(&mut self, pm: &Pixmap, bands: Range<usize>) {
        // The same code compiled for AVX2 (8 floats per instruction) when the CPU has it
        #[cfg(target_arch = "x86_64")]
        if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma") {
            // SAFETY: the CPU supports the features enabled for encode_avx2
            return unsafe { self.encode_avx2(pm, bands) };
        }
        self.encode_impl(pm, bands)
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2,fma,bmi2,lzcnt")]
    unsafe fn encode_avx2(&mut self, pm: &Pixmap, bands: Range<usize>) {
        self.encode_impl(pm, bands)
    }

    #[inline(always)]
    fn encode_impl(&mut self, pm: &Pixmap, bands: Range<usize>) {
        let px = pm.data(); // RGBA, premultiplied = straight for opaque pixels
        let lw = pm.width() as usize; // landscape width = portrait height
        let lh = pm.height() as usize;
        debug_assert!(lw == HEIGHT && lh == WIDTH);
        let mut blocks = [[0f32; 64]; 3];
        for b in bands.start.min(BANDS)..bands.end.min(BANDS) {
            let h = band_hash(px, lw, b);
            if h == self.hashes[b] && !self.bands[b].is_empty() {
                continue; // same pixels as last time: keep the bytes
            }
            self.hashes[b] = h;
            let mut out = std::mem::take(&mut self.bands[b]);
            out.clear();
            let mut bits = Bits { out: &mut out, acc: 0, n: 0 };
            let mut pred = [0i32; 3];
            for m in 0..MCUS {
                // portrait pixel (x, y) = landscape (y, lh - 1 - x)
                for yy in 0..8 {
                    let lx = b * 8 + yy;
                    for xx in 0..8 {
                        let ly = lh - 1 - (m * 8 + xx);
                        let i = (ly * lw + lx) * 4;
                        let (r, g, bl) = (px[i] as f32, px[i + 1] as f32, px[i + 2] as f32);
                        let k = yy * 8 + xx;
                        blocks[0][k] = 0.299 * r + 0.587 * g + 0.114 * bl - 128.0;
                        blocks[1][k] = -0.168_736 * r - 0.331_264 * g + 0.5 * bl;
                        blocks[2][k] = 0.5 * r - 0.418_688 * g - 0.081_312 * bl;
                    }
                }
                for (c, blk) in blocks.iter_mut().enumerate() {
                    let t = (c > 0) as usize;
                    fdct(blk);
                    let div = &self.div[t];
                    let q: [i32; 64] = std::array::from_fn(|z| {
                        let n = ZIGZAG[z];
                        (blk[n] * div[n]).round() as i32
                    });
                    let (cat, extra) = category(q[0] - pred[c]);
                    pred[c] = q[0];
                    let (code, len) = self.dc[t].0[cat as usize];
                    bits.put(code as u32, len as u32);
                    if cat > 0 {
                        bits.put(extra, cat);
                    }
                    let mut run = 0;
                    for &v in &q[1..] {
                        if v == 0 {
                            run += 1;
                            continue;
                        }
                        while run > 15 {
                            let (code, len) = self.ac[t].0[0xF0];
                            bits.put(code as u32, len as u32);
                            run -= 16;
                        }
                        let (cat, extra) = category(v);
                        let (code, len) = self.ac[t].0[(run << 4 | cat) as usize];
                        bits.put(code as u32, len as u32);
                        bits.put(extra, cat);
                        run = 0;
                    }
                    if run > 0 {
                        let (code, len) = self.ac[t].0[0x00]; // EOB
                        bits.put(code as u32, len as u32);
                    }
                }
            }
            bits.flush();
            self.bands[b] = out;
        }
    }

    /// The whole frame: header, bands separated by restart markers, EOI.
    pub fn frame(&mut self) -> &[u8] {
        self.frame.clear();
        self.frame.extend_from_slice(&self.header);
        for (b, band) in self.bands.iter().enumerate() {
            self.frame.extend_from_slice(band);
            if b + 1 < BANDS {
                self.frame.extend_from_slice(&[0xFF, 0xD0 + (b % 8) as u8]);
            }
        }
        self.frame.extend_from_slice(&[0xFF, 0xD9]);
        &self.frame
    }
}

/// Hash of a band's pixels: landscape columns 8b..8b+8 of every row (32 bytes per row).
#[inline(always)]
fn band_hash(px: &[u8], lw: usize, b: usize) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for row in px.chunks_exact(lw * 4) {
        for w in row[b * 32..b * 32 + 32].chunks_exact(8) {
            h = (h ^ u64::from_le_bytes(w.try_into().unwrap())).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// Bands covering landscape columns `x0..x1`.
pub fn bands_for(x0: f32, x1: f32) -> Range<usize> {
    let a = (x0.max(0.0) as usize) / 8;
    let b = ((x1.max(0.0) as usize) + 7) / 8;
    a.min(BANDS)..b.min(BANDS)
}
