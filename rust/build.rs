//! The animated AI mascots are personal-use only (their art belongs to Anthropic / OpenAI), so
//! they are not in this repository. If a mascot folder with `mascot.rs` and its art is found,
//! it is built in; otherwise the dashboard builds without mascots.
//!
//! Looked up in: `$TURZX_MASCOTS`, `../assets/mascots`, `../../assets/mascots` (a private
//! repository that has this one as a submodule).

use std::path::PathBuf;

const FILES: [&str; 5] = ["mascot.rs", "clawd.png", "clawd.json", "codex.png", "codex.json"];

fn main() {
    println!("cargo::rustc-check-cfg=cfg(mascots)");
    println!("cargo::rerun-if-env-changed=TURZX_MASCOTS");
    let candidates = std::env::var_os("TURZX_MASCOTS")
        .map(PathBuf::from)
        .into_iter()
        .chain(["../assets/mascots", "../../assets/mascots"].map(PathBuf::from));
    for dir in candidates {
        println!("cargo::rerun-if-changed={}", dir.display());
        if FILES.iter().all(|f| dir.join(f).is_file()) {
            let abs = dir.canonicalize().unwrap_or(dir);
            // include!/include_bytes! want a plain path: drop the \\?\ prefix, use forward slashes
            let abs = abs.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/");
            println!("cargo::rustc-cfg=mascots");
            println!("cargo::rustc-env=TURZX_MASCOTS_DIR={abs}");
            return;
        }
    }
}
