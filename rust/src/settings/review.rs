//! Capture this application's own native window with Win32's WM_PRINT renderer.
use std::path::Path;
use anyhow::{Result, ensure};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub fn capture(hwnd: HWND, path: &Path) -> Result<()> {
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect)?;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        ensure!(width > 0 && height > 0, "Empty settings window");
        let dc = CreateCompatibleDC(None);
        ensure!(!dc.0.is_null(), "Create screenshot DC");
        let info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width, biHeight: -height, biPlanes: 1, biBitCount: 32,
            biCompression: BI_RGB.0, ..Default::default()
        }, ..Default::default() };
        let mut pixels = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut pixels, None, 0) {
            Ok(bitmap) => bitmap,
            Err(error) => { let _ = DeleteDC(dc); return Err(error.into()); }
        };
        let old = SelectObject(dc, bitmap.into());
        SendMessageW(hwnd, WM_PRINT, Some(WPARAM(dc.0 as usize)), Some(LPARAM((PRF_CHECKVISIBLE | PRF_NONCLIENT | PRF_CLIENT | PRF_CHILDREN | PRF_ERASEBKGND) as isize)));
        let _ = GdiFlush();
        let bytes = std::slice::from_raw_parts(pixels.cast::<u8>(), (width * height * 4) as usize);
        let mut image = tiny_skia::Pixmap::new(width as u32, height as u32).unwrap();
        for (source, dest) in bytes.chunks_exact(4).zip(image.data_mut().chunks_exact_mut(4)) {
            dest.copy_from_slice(&[source[2], source[1], source[0], 255]);
        }
        SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        image.save_png(path)?;
        Ok(())
    }
}
