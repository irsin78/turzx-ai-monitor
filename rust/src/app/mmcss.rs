//! Scope multimedia scheduling to the render/send thread; EcoQoS stays enabled.
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
};
use windows::core::w;

pub(super) struct Multimedia(Option<HANDLE>);

impl Multimedia {
    pub fn register() -> Self {
        let mut index = 0;
        // Games covers interactive animation/rendering; no audio priority boost.
        match unsafe { AvSetMmThreadCharacteristicsW(w!("Games"), &mut index) } {
            Ok(handle) => {
                log::info!("MMCSS Games registered for render/send thread (EcoQoS retained)");
                Self(Some(handle))
            }
            Err(e) => {
                log::warn!("MMCSS Games unavailable: {e}; continuing with EcoQoS");
                Self(None)
            }
        }
    }
}

impl Drop for Multimedia {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            let _ = unsafe { AvRevertMmThreadCharacteristics(handle) };
        }
    }
}
