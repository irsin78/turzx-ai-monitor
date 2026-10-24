//! H.264 for the panel's own player, with the encoder that comes with Windows (Media Foundation,
//! Microsoft H.264 Encoder MFT; no extra software).
//!
//! The panel plays what the vendor app produces: a raw Annex B stream, portrait 480x1920, no
//! B-frames. Frames are drawn landscape (1920x480) like the dashboard and turned to portrait here,
//! straight into NV12. Baseline profile has no B-frames by definition.

use std::mem::ManuallyDrop;

use anyhow::{Context, Result};
use tiny_skia::Pixmap;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

/// Portrait size of the panel's video.
pub const W: u32 = 480;
pub const H: u32 = 1920;

pub struct H264 {
    mft: IMFTransform,
    fps: u32,
    frames: i64,
    /// turn the picture 180 degrees (panel mounted upside down)
    flip: bool,
    nv12: Vec<u8>,
    out: Vec<u8>,
}

impl H264 {
    pub fn new(fps: u32, bitrate: u32, flip: bool) -> Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED); // already initialized is fine
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).context("MFStartup")?;
            let mft: IMFTransform = CoCreateInstance(&CLSID_MSH264EncoderMFT, None, CLSCTX_INPROC_SERVER).context("H.264 encoder")?;
            let video = |subtype: &windows::core::GUID| -> Result<IMFMediaType> {
                let t = MFCreateMediaType()?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
                t.SetUINT64(&MF_MT_FRAME_SIZE, ((W as u64) << 32) | H as u64)?;
                t.SetUINT64(&MF_MT_FRAME_RATE, ((fps as u64) << 32) | 1)?;
                t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1)?;
                t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                Ok(t)
            };
            let out = video(&MFVideoFormat_H264)?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Base.0 as u32)?;
            mft.SetOutputType(0, &out, 0).context("encoder output type")?;
            mft.SetInputType(0, &video(&MFVideoFormat_NV12)?, 0).context("encoder input type")?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            Ok(H264 { mft, fps, frames: 0, flip, nv12: vec![0; (W * H * 3 / 2) as usize], out: Vec::new() })
        }
    }

    /// Add one landscape 1920x480 frame.
    pub fn push(&mut self, frame: &Pixmap) -> Result<()> {
        anyhow::ensure!(frame.width() == H && frame.height() == W, "frames are 1920x480");
        to_nv12(frame, self.flip, &mut self.nv12);
        unsafe {
            let buf = MFCreateMemoryBuffer(self.nv12.len() as u32)?;
            let mut p = std::ptr::null_mut();
            buf.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(self.nv12.as_ptr(), p, self.nv12.len());
            buf.Unlock()?;
            buf.SetCurrentLength(self.nv12.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buf)?;
            let tick = 10_000_000 / self.fps as i64; // 100 ns units
            sample.SetSampleTime(self.frames * tick)?;
            sample.SetSampleDuration(tick)?;
            self.frames += 1;
            self.mft.ProcessInput(0, &sample, 0).context("encoder input")?;
        }
        self.drain()
    }

    /// The finished stream.
    pub fn finish(mut self) -> Result<Vec<u8>> {
        unsafe {
            self.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)?;
            self.mft.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)?;
        }
        self.drain()?;
        Ok(std::mem::take(&mut self.out))
    }

    /// Collect whatever the encoder has ready.
    fn drain(&mut self) -> Result<()> {
        unsafe {
            let size = self.mft.GetOutputStreamInfo(0)?.cbSize.max(1 << 20);
            loop {
                let buf = MFCreateMemoryBuffer(size)?;
                let sample = MFCreateSample()?;
                sample.AddBuffer(&buf)?;
                let mut out = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(Some(sample.clone())), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
                let mut status = 0u32;
                let r = self.mft.ProcessOutput(0, &mut out, &mut status);
                ManuallyDrop::drop(&mut out[0].pSample);
                ManuallyDrop::drop(&mut out[0].pEvents);
                match r {
                    Ok(()) => {
                        let data = sample.ConvertToContiguousBuffer()?;
                        let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
                        data.Lock(&mut p, None, Some(&mut len))?;
                        self.out.extend_from_slice(std::slice::from_raw_parts(p, len as usize));
                        data.Unlock()?;
                    }
                    Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                    Err(e) => return Err(e).context("encoder output"),
                }
            }
        }
    }
}

impl Drop for H264 {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
        }
    }
}

/// Landscape RGBA frame -> portrait NV12 (BT.601, limited range), turned 90 degrees clockwise
/// like the dashboard frames (180 more when flipped).
fn to_nv12(frame: &Pixmap, flip: bool, out: &mut [u8]) {
    let (lw, lh) = (frame.width() as usize, frame.height() as usize); // 1920, 480
    let (w, h) = (lh, lw); // portrait 480 x 1920
    let px = frame.data();
    let at = |x: usize, y: usize| {
        // portrait (x, y) = landscape (y, lh-1-x); flipped: (lw-1-y, x)
        let (lx, ly) = if flip { (lw - 1 - y, x) } else { (y, lh - 1 - x) };
        let i = (ly * lw + lx) * 4;
        (px[i] as i32, px[i + 1] as i32, px[i + 2] as i32)
    };
    let (luma, chroma) = out.split_at_mut(w * h);
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = at(x, y);
            luma[y * w + x] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
        }
    }
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let mut s = (0, 0, 0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (r, g, b) = at(x + dx, y + dy);
                s = (s.0 + r, s.1 + g, s.2 + b);
            }
            let (r, g, b) = (s.0 / 4, s.1 / 4, s.2 / 4);
            let i = (y / 2) * w + x;
            chroma[i] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128) as u8;
            chroma[i + 1] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128) as u8;
        }
    }
}
