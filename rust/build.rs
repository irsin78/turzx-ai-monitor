//! The animated AI mascots are personal-use only (their art belongs to Anthropic / OpenAI), so
//! they are not in this repository. If a mascot folder with `mascot.rs` and its art is found,
//! it is built in; otherwise the dashboard builds without mascots.
//!
//! Looked up in: `$TURZX_MASCOTS` (`off` builds without them), `../assets/mascots`, `../../assets/mascots` (a private
//! repository that has this one as a submodule).

use std::path::PathBuf;

const FILES: [&str; 5] = ["mascot.rs", "clawd.png", "clawd.json", "codex.png", "codex.json"];

fn main() {
    resources();
    println!("cargo::rustc-check-cfg=cfg(mascots)");
    println!("cargo::rerun-if-env-changed=TURZX_MASCOTS");
    if std::env::var("TURZX_MASCOTS").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
        return; // build the public flavor even when the art is around (screenshots, releases)
    }
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

/// The app icon and the manifest (themed controls in the settings window), compiled with the
/// Windows SDK's rc.exe. The icon is `app.ico` from the mascot folder when there is one,
/// else `assets/app.ico`. Without the SDK the exe is built without them.
fn resources() {
    if std::env::var("TARGET").as_deref() != Ok("x86_64-pc-windows-msvc") {
        return;
    }
    println!("cargo::rerun-if-changed=app.manifest");
    println!("cargo::rerun-if-changed=assets/app.ico");
    println!("cargo::rerun-if-changed=../assets/mascots/app.ico");
    println!("cargo::rerun-if-changed=../../assets/mascots/app.ico");
    println!("cargo::rerun-if-env-changed=WindowsSdkDir");
    let path = |p: &std::path::Path| {
        let abs = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        abs.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/")
    };
    let mut rc = format!("1 24 \"{}\"\n", path(std::path::Path::new("app.manifest")));
    let mascots = !std::env::var("TURZX_MASCOTS").is_ok_and(|v| v.eq_ignore_ascii_case("off"));
    let icon = ["../assets/mascots/app.ico", "../../assets/mascots/app.ico"]
        .map(PathBuf::from)
        .into_iter()
        .filter(|_| mascots)
        .chain([PathBuf::from("assets/app.ico")])
        .find(|p| p.is_file());
    if let Some(icon) = icon {
        rc.push_str(&format!("1 ICON \"{}\"\n", path(&icon)));
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out.join("app.rc"), rc).unwrap();
    let sdk = std::env::var_os("WindowsSdkDir").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10"));
    let mut compilers: Vec<PathBuf> = std::fs::read_dir(sdk.join("bin"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path().join("x64/rc.exe"))
        .filter(|p| p.is_file())
        .collect();
    compilers.sort();
    let Some(compiler) = compilers.last() else {
        println!("cargo::warning=Windows SDK rc.exe not found: building without icon and manifest");
        return;
    };
    let res = out.join("app.res");
    let ok = std::process::Command::new(compiler).arg("/nologo").arg("/fo").arg(&res).arg(out.join("app.rc")).status().is_ok_and(|s| s.success());
    assert!(ok, "rc.exe failed");
    println!("cargo::rustc-link-arg={}", res.display());
}
