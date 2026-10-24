//! Video files (MP4 and whatever else Windows can play) as frames, with the decoder that comes
//! with Windows (Media Foundation source reader, converted to RGB by its video processor).

use anyhow::{Context, Result};
use tiny_skia::Pixmap;
use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

pub struct Video {
    reader: IMFSourceReader,
    pub width: u32,
    pub height: u32,
    stride: i32,
}

const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

impl Video {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).context("MFStartup")?;
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 1)?;
            let attrs = attrs.context("attributes")?;
            attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
            let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), &attrs).with_context(|| format!("open {}", path.display()))?;
            let want = MFCreateMediaType()?;
            want.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            want.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            reader.SetCurrentMediaType(STREAM, None, &want).context("RGB output")?;
            let got = reader.GetCurrentMediaType(STREAM)?;
            let size = got.GetUINT64(&MF_MT_FRAME_SIZE)?;
            let (width, height) = ((size >> 32) as u32, size as u32);
            let stride = got.GetUINT32(&MF_MT_DEFAULT_STRIDE).map_or(width as i32 * 4, |s| s as i32);
            Ok(Video { reader, width, height, stride })
        }
    }

    /// Length in seconds, if the file says.
    pub fn duration(&self) -> Option<f64> {
        use windows::Win32::System::Com::StructuredStorage::PropVariantToUInt64;
        unsafe {
            let v = self.reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION).ok()?;
            PropVariantToUInt64(&v).ok().map(|t| t as f64 / 1e7)
        }
    }

    /// The next frame and its time in seconds, or None at the end.
    pub fn next(&mut self) -> Result<Option<(f64, Pixmap)>> {
        unsafe {
            loop {
                let (mut flags, mut time, mut sample) = (0u32, 0i64, None);
                self.reader.ReadSample(STREAM, 0, None, Some(&mut flags), Some(&mut time), Some(&mut sample))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    return Ok(None);
                }
                let Some(sample) = sample else { continue }; // a gap or a format change
                let buf = sample.ConvertToContiguousBuffer()?;
                let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
                buf.Lock(&mut p, None, Some(&mut len))?;
                let data = std::slice::from_raw_parts(p, len as usize);
                let mut pm = Pixmap::new(self.width, self.height).context("frame size")?;
                let row = self.width as usize * 4;
                let pitch = self.stride.unsigned_abs() as usize;
                for (y, dst) in pm.data_mut().chunks_exact_mut(row).enumerate() {
                    // a negative stride means bottom-up rows
                    let sy = if self.stride < 0 { self.height as usize - 1 - y } else { y };
                    let Some(src) = data.get(sy * pitch..sy * pitch + row) else { break };
                    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                        d.copy_from_slice(&[s[2], s[1], s[0], 255]); // BGRX -> RGBA
                    }
                }
                buf.Unlock()?;
                return Ok(Some((time as f64 / 1e7, pm)));
            }
        }
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
        }
    }
}
