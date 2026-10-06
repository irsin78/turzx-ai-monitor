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
    let mut cv = match Pixmap::decode_png(STANDBY[i]) {
        Ok(pm) => Canvas { pm },
        Err(e) => {
            log::warn!("standby page {i}: {e}");
            Canvas::background()
        }
    };
    // subtitle next to the accent bar, in the screen language (layout from standby_pages.py)
    cv.text(1082.0, 300.0, i18n::get().standby[i], ko(30.0, false), [138, 151, 184]);
    cv
}
