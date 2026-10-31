//! Log file: %LOCALAPPDATA%\TurzxDashboard\dashboard.log; past 1 MB it moves to dashboard.1.log
//! (replacing the one before) and logging goes on in a new file.

use std::io::Write;
use std::path::PathBuf;

const LIMIT: u64 = 1_000_000;

/// Log to %LOCALAPPDATA%\TurzxDashboard\dashboard.log (and stderr when there is a console).
pub fn init() {
    let dir = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("TurzxDashboard");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("dashboard.log");
    let mut b = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,tiny_skia=error"));
    if let Some(log) = Log::open(path) {
        b.target(env_logger::Target::Pipe(Box::new(log)));
    }
    b.init();
}

/// The log file, moved aside when it grows past `LIMIT`. Several processes append to it (the
/// supervisor, the dashboard, commands); one that moves it carries on in a new file while the
/// others finish in the old one until they start again.
struct Log {
    path: PathBuf,
    file: std::fs::File,
    size: u64,
}

impl Log {
    fn open(path: PathBuf) -> Option<Self> {
        if std::fs::metadata(&path).is_ok_and(|m| m.len() >= LIMIT) {
            let _ = std::fs::rename(&path, path.with_file_name("dashboard.1.log"));
        }
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()?;
        let size = file.metadata().map_or(0, |m| m.len());
        Some(Log { path, file, size })
    }
}

impl Write for Log {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        self.size += buf.len() as u64;
        if self.size >= LIMIT {
            // the file may also have grown through the other processes: check the real size
            let real = std::fs::metadata(&self.path).map_or(self.size, |m| m.len());
            if real >= LIMIT {
                if let Some(fresh) = Log::open(self.path.clone()) {
                    *self = fresh;
                }
            } else {
                self.size = real;
            }
        }
        self.file.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
