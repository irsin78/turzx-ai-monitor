//! Start-up picture. When the panel powers on, its firmware shows `/usr/data/boot.jpg` if that
//! file exists, else its own `/media/logo.jpg`. The firmware cannot pick among several files or
//! rename/copy one, so the dashboard uploads a randomly chosen picture from the boot folder each
//! time it connects: the panel shows it the next time it powers on.
//!
//! Folder: `[boot] images`, or `%LOCALAPPDATA%\TurzxDashboard\boot`; `.jpg`/`.jpeg`/`.png`,
//! scaled to cover 1920x480. An empty or missing folder leaves the panel's file alone.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

use crate::panel::lcd;

/// Where the firmware looks for a user start-up picture.
pub const PANEL_PATH: &str = "/usr/data/boot.jpg";

const W: u32 = 1920;
const H: u32 = 480;

fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("TurzxDashboard")
}

/// The boot picture folder.
pub fn dir() -> PathBuf {
    let dir = crate::config::get().boot.images.trim().to_string();
    if dir.is_empty() { data_dir().join("boot") } else { PathBuf::from(dir) }
}

/// Pictures in the folder, sorted by name.
pub fn pictures(dir: &Path) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png"))
        })
        .collect();
    list.sort();
    list
}

/// A random picture, not the one used last time when there is a choice.
pub fn pick<'a>(list: &'a [PathBuf], last: &str, seed: u64) -> Option<&'a PathBuf> {
    let name = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let others: Vec<&PathBuf> = list.iter().filter(|p| list.len() == 1 || name(p) != last).collect();
    if others.is_empty() {
        return None;
    }
    Some(others[(seed % others.len() as u64) as usize])
}

fn load(path: &Path) -> Result<Pixmap> {
    let bytes = std::fs::read(path).with_context(|| path.display().to_string())?;
    if bytes.starts_with(b"\x89PNG") {
        return Pixmap::decode_png(&bytes).with_context(|| format!("{}: bad PNG", path.display()));
    }
    let mut dec = zune_jpeg::JpegDecoder::new(std::io::Cursor::new(bytes));
    let rgb = dec.decode().map_err(|e| anyhow::anyhow!("{}: {e:?}", path.display()))?;
    let info = dec.info().context("jpeg size")?;
    let (w, h) = (info.width as u32, info.height as u32);
    anyhow::ensure!(rgb.len() == (w * h * 3) as usize, "{}: not RGB", path.display());
    let mut pm = Pixmap::new(w, h).context("image size")?;
    for (d, s) in pm.data_mut().chunks_exact_mut(4).zip(rgb.chunks_exact(3)) {
        d.copy_from_slice(&[s[0], s[1], s[2], 255]);
    }
    Ok(pm)
}

/// The picture scaled to cover 1920x480 (centred, cropped), on black, as landscape RGB.
pub fn landscape(path: &Path) -> Result<Vec<u8>> {
    let src = load(path)?;
    let scale = (W as f32 / src.width() as f32).max(H as f32 / src.height() as f32);
    let (dx, dy) = ((W as f32 - src.width() as f32 * scale) / 2.0, (H as f32 - src.height() as f32 * scale) / 2.0);
    let mut out = Pixmap::new(W, H).context("frame")?;
    out.fill(tiny_skia::Color::BLACK);
    let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
    out.draw_pixmap(0, 0, src.as_ref(), &paint, Transform::from_row(scale, 0.0, 0.0, scale, dx, dy), None);
    Ok(out.data().chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect())
}

/// Upload a random picture as the next start-up picture. Returns its file name, or `None`
/// when the folder has no pictures.
pub fn update(panel: &mut lcd::Lcd) -> Result<Option<String>> {
    let list = pictures(&dir());
    let last_file = data_dir().join("boot_last.txt");
    let last = std::fs::read_to_string(&last_file).unwrap_or_default();
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64 / 1000);
    let Some(path) = pick(&list, last.trim(), seed) else { return Ok(None) };
    let jpeg = lcd::landscape_jpeg(&landscape(path)?)?;
    panel.put_file(PANEL_PATH, &jpeg)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let _ = std::fs::write(&last_file, &name);
    Ok(Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_avoids_last() {
        let list: Vec<PathBuf> = ["boot1.png", "boot2.png", "boot3.png"].iter().map(PathBuf::from).collect();
        for seed in 0..20 {
            assert_ne!(pick(&list, "boot2.png", seed).unwrap(), &PathBuf::from("boot2.png"));
        }
        let one = vec![PathBuf::from("only.jpg")];
        assert_eq!(pick(&one, "only.jpg", 7), Some(&one[0]));
        assert_eq!(pick(&[], "", 0), None);
    }
}
