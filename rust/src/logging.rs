//! Log file: %LOCALAPPDATA%\TurzxDashboard\dashboard.log.

use std::io::Write;

/// Log to %LOCALAPPDATA%\TurzxDashboard\dashboard.log (and stderr when there is a console).
pub fn init() {
    struct Tee(std::fs::File);
    impl Write for Tee {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let _ = std::io::stderr().write_all(buf);
            self.0.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }
    let dir = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("TurzxDashboard");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("dashboard.log");
    // keep the log small: start over once it passes 1 MB
    let append = std::fs::metadata(&path).map(|m| m.len() < 1_000_000).unwrap_or(false);
    let mut b = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,tiny_skia=error"));
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(append).write(true).truncate(!append).open(&path) {
        b.target(env_logger::Target::Pipe(Box::new(Tee(f))));
    }
    b.init();
}
