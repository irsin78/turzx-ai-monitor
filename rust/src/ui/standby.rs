//! Standby screens, shown when Windows shuts down (they stay on the panel until the next start).

use super::*;

/// Standby screens built by ../scripts/standby_pages.py (art generated with Codex).
pub(super) static STANDBY: [&[u8]; 3] = [
    include_bytes!("../../../assets/standby/page_1.png"),
    include_bytes!("../../../assets/standby/page_2.png"),
    include_bytes!("../../../assets/standby/page_3.png"),
];

/// One of the standby screens at random (shown at shutdown; stays on the panel until the next start).
pub fn standby() -> Canvas {
    let pick = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) as usize % STANDBY.len();
    standby_page(pick)
}

pub fn standby_page(i: usize) -> Canvas {
    match Pixmap::decode_png(STANDBY[i]) {
        Ok(pm) => Canvas { pm },
        Err(e) => {
            log::warn!("standby page {i}: {e}");
            Canvas::background()
        }
    }
}
