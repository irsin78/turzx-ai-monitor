//! `hooks install|uninstall|status`: add the agent-status hooks to Claude Code's
//! ~/.claude/settings.json and wrap Codex's notify in ~/.codex/config.toml (backups first,
//! atomic replacement; uninstall restores the original notify). agent-event changes nothing.
use std::path::{Path, PathBuf};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use toml::de::DeTable;
use crate::agent_status::MARKER;

const EVENTS: [&str; 7] = ["Notification", "PermissionRequest", "PreToolUse", "Stop", "UserPromptSubmit", "PostToolUse", "SessionEnd"];
/// PreToolUse runs for every tool: only the ones that wait for the user start the hook.
const QUESTION_TOOLS: &str = "AskUserQuestion|ExitPlanMode|mcp__ccd_turn__wrap_up";

fn owned_command(s: &str) -> bool {
    s.contains("turzx-dashboard.exe") && s.contains(" agent-event claude ") && s.ends_with(MARKER)
}
fn owned_notify(argv: &[String]) -> bool {
    argv.first().is_some_and(|p| p.replace('\\', "/").ends_with("/turzx-dashboard.exe"))
        && argv.get(1).is_some_and(|s| s == "agent-event") && argv.get(2).is_some_and(|s| s == "codex")
        && argv.last().is_some_and(|s| s == MARKER)
}
pub fn claude_command(exe: &Path, port: u16) -> String {
    // POSIX single quoting works in Git Bash, including spaces, $, ` and apostrophes.
    let exe = exe.to_string_lossy().replace('\\', "/").replace('\'', "'\"'\"'");
    format!("'{exe}' agent-event claude --port {port} {MARKER}")
}
fn edit_claude(text: &str, exe: &Path, port: u16, install: bool) -> Result<String> {
    if !install && text.trim().is_empty() { return Ok(text.into()); }
    let mut root: Value = if text.trim().is_empty() { json!({}) } else { serde_json::from_str(text)? };
    ensure!(root.is_object(), "Claude settings must be an object");
    if let Some(hooks) = root.get_mut("hooks") {
        let hooks = hooks.as_object_mut().context("Claude hooks must be an object")?;
        for event in EVENTS {
            if let Some(groups) = hooks.get_mut(event) {
                let groups = groups.as_array_mut().context("Claude event must be an array")?;
                groups.retain_mut(|group| {
                    if let Some(inner) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                        let before = inner.len();
                        inner.retain(|h| !h["command"].as_str().is_some_and(owned_command));
                        // a group of ours left empty goes, matcher and all
                        return before == inner.len() || !inner.is_empty();
                    }
                    true
                });
            }
        }
    }
    if install {
        let hooks = root.as_object_mut().unwrap().entry("hooks").or_insert_with(|| json!({})).as_object_mut().context("Claude hooks must be an object")?;
        for event in EVENTS {
            hooks.entry(event).or_insert_with(|| json!([])).as_array_mut().context("Claude event must be an array")?
                .push(match event {
                    "PreToolUse" => json!({"matcher":QUESTION_TOOLS,"hooks":[{"type":"command","command":claude_command(exe, port),"timeout":2}]}),
                    _ => json!({"hooks":[{"type":"command","command":claude_command(exe, port),"timeout":2}]}),
                });
        }
    }
    // Avoid gratuitous rewrites, including a second installation.
    if !text.trim().is_empty() && serde_json::from_str::<Value>(text)? == root { return Ok(text.into()); }
    Ok(serde_json::to_string_pretty(&root)? + "\n")
}

fn notify(text: &str) -> Result<Option<(std::ops::Range<usize>, String, Vec<String>)>> {
    let root = DeTable::parse(text)?;
    let Some((_, value)) = root.get_ref().iter().find(|(k, _)| k.get_ref().as_ref() == "notify") else { return Ok(None); };
    let range = value.span();
    let raw = text[range.clone()].to_owned();
    let table: toml::Table = toml::from_str(&format!("notify = {raw}"))?;
    let argv: Vec<String> = table["notify"].as_array().context("Codex notify must be an argv array")?.iter()
        .map(|v| v.as_str().map(str::to_owned).context("Codex notify argv must contain strings")).collect::<Result<_>>()?;
    Ok(Some((range, raw, argv)))
}
fn previous(argv: &[String]) -> Result<&str> {
    let raw = argv.iter().position(|s| s == "--previous-notify").and_then(|i| argv.get(i + 1))
        .context("Our notify has no saved previous command; leaving it untouched")?;
    if !raw.is_empty() { let _ = notify(&format!("notify = {raw}"))?; }
    Ok(raw)
}
fn edit_codex(text: &str, exe: &Path, port: u16, install: bool) -> Result<String> {
    let existing = notify(text)?;
    let saved = match &existing { Some((_, _, argv)) if owned_notify(argv) => previous(argv)?.to_owned(),
        Some((_, raw, _)) => raw.clone(), None => String::new() };
    let replacement = if install {
        let argv = vec![exe.to_string_lossy().into_owned(), "agent-event".into(), "codex".into(), "--port".into(),
            port.to_string(), "--previous-notify".into(), saved, MARKER.into()];
        toml::Value::Array(argv.into_iter().map(toml::Value::String).collect()).to_string()
    } else {
        if !existing.as_ref().is_some_and(|(_, _, argv)| owned_notify(argv)) { return Ok(text.into()); }
        saved
    };
    let mut result = text.to_owned();
    match existing {
        Some((range, _, _)) if !replacement.is_empty() => result.replace_range(range, &replacement),
        Some((range, _, _)) => {
            // Only our inserted top-level assignment, including its newline, is removed.
            let start = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
            let end = text[range.end..].find('\n').map_or(text.len(), |i| range.end + i + 1);
            result.replace_range(start..end, "");
        }
        None if install => {
            let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
            result = format!("notify = {replacement}{newline}{text}");
        }
        None => {}
    }
    let _: toml::Table = toml::from_str(&result)?;
    Ok(result)
}
fn read(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) { Ok(s) => Ok(s), Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()), Err(e) => Err(e.into()) }
}
fn write(path: &Path, before: &str, after: &str) -> Result<()> {
    if before == after { return Ok(()); }
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let mut backup = path.as_os_str().to_owned(); backup.push(".bak");
    std::fs::write(PathBuf::from(backup), before).context("Write hook configuration backup")?;
    let temp = path.with_extension(format!("turzx-{}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
    use std::io::Write;
    let result = (|| -> Result<()> {
        file.write_all(after.as_bytes())?; file.sync_all()?; drop(file);
        ensure!(read(path)? == before, "Configuration changed during hook installation; retry after reviewing it");
        use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
        use windows::core::HSTRING;
        unsafe { MoveFileExW(&HSTRING::from(temp.as_os_str()), &HSTRING::from(path.as_os_str()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) }?;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(temp); }
    result
}

pub fn edit_paths(claude: &Path, codex: &Path, exe: &Path, port: u16, install: bool) -> Result<()> {
    ensure!(port != 0, "Agent status UDP port must be 1..65535");
    let (c, x) = (read(claude)?, read(codex)?);
    // Validate both before writing either, so invalid unrelated config is untouched.
    let (new_c, new_x) = (edit_claude(&c, exe, port, install)?, edit_codex(&x, exe, port, install)?);
    write(claude, &c, &new_c)?;
    write(codex, &x, &new_x)
}
pub fn command(action: &str, args: &[String]) -> Result<String> {
    let home = PathBuf::from(std::env::var_os("USERPROFILE").context("USERPROFILE is unset")?);
    let option = |key| args.iter().position(|s| s == key).and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let claude = option("--claude-config").unwrap_or_else(|| home.join(".claude/settings.json"));
    let codex = option("--codex-config").unwrap_or_else(|| home.join(".codex/config.toml"));
    let exe = crate::install::install_dir().join("turzx-dashboard.exe");
    match action {
        "install" | "uninstall" => {
            edit_paths(&claude, &codex, &exe, crate::config::get().agent_status_port, action == "install")?;
            Ok(format!("Agent hooks {action} complete.\nClaude: {}\nCodex: {}", claude.display(), codex.display()))
        }
        "status" => {
            let c: Value = serde_json::from_str(&read(&claude)?).unwrap_or(json!({}));
            let count = EVENTS.iter().filter(|e| c["hooks"][**e].as_array().is_some_and(|groups| groups.iter().any(|g|
                g["hooks"].as_array().is_some_and(|hooks| hooks.iter().any(|h| h["command"].as_str().is_some_and(owned_command)))))).count();
            let x = notify(&read(&codex)?)?.is_some_and(|(_, _, argv)| owned_notify(&argv));
            Ok(format!("Claude: {count}/{} hooks; Codex notify wrapper: {x}\nInstalled exe: {}", EVENTS.len(), exe.display()))
        }
        _ => anyhow::bail!("usage: hooks install|uninstall|status [--claude-config PATH] [--codex-config PATH]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn exe() -> &'static Path { Path::new("C:/Users/Test User/Programs/TurzxDashboard/turzx-dashboard.exe") }
    #[test]
    fn claude_preserves_existing_hooks_and_all_other_values() {
        let input = r#"{"custom":[1,true],"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"existing"}]}],"Stop":[{"matcher":"x","hooks":[{"type":"command","command":"stop"}]}]}}"#;
        let after = edit_claude(input, exe(), 47135, true).unwrap();
        assert_eq!(edit_claude(&after, exe(), 47135, true).unwrap(), after);
        let uninstalled: Value = serde_json::from_str(&edit_claude(&after, exe(), 47135, false).unwrap()).unwrap();
        let original: Value = serde_json::from_str(input).unwrap();
        assert_eq!(uninstalled["custom"], original["custom"]);
        assert_eq!(uninstalled["hooks"]["SessionStart"], original["hooks"]["SessionStart"]);
        assert_eq!(uninstalled["hooks"]["Stop"], original["hooks"]["Stop"]);
        let mut mixed: Value = serde_json::from_str(&after).unwrap();
        mixed["hooks"]["Notification"][0]["hooks"].as_array_mut().unwrap().push(json!({"type":"command","command":"added later"}));
        let clean: Value = serde_json::from_str(&edit_claude(&mixed.to_string(), exe(), 1234, false).unwrap()).unwrap();
        assert_eq!(clean["hooks"]["Notification"][0]["hooks"][0]["command"], "added later");
        assert!(claude_command(Path::new("C:/a'b/$x/`file`/turzx-dashboard.exe"), 47135).contains("'\"'\"'"));
    }
    #[test]
    fn codex_restores_exact_notify_and_comments() {
        for input in ["# top\r\nnotify = [ \"C:\\\\helpers\\\\codex-computer-use.exe\", \"turn-ended\" ] # existing\r\nmodel = 'x'\r\n[projects.'C:\\a']\r\ntrust_level = 'trusted'\r\n",
            "# top\nnotify = [\n 'helper', # inner comment\n 'turn-ended',\n]\nmodel = 'x'\n", "# no notify\nmodel = 'x'\n[other]\nx = 3\n", "", "notify = []\n"] {
            let after = edit_codex(input, exe(), 47135, true).unwrap();
            assert_eq!(edit_codex(&after, exe(), 47135, true).unwrap(), after);
            assert_eq!(edit_codex(&after, exe(), 47135, false).unwrap(), input);
            let changed = edit_codex(&after, exe(), 12345, true).unwrap();
            assert_eq!(edit_codex(&changed, exe(), 12345, false).unwrap(), input);
        }
        let changed = "notify = ['user-replaced-it']\n";
        assert_eq!(edit_codex(changed, exe(), 47135, false).unwrap(), changed);
        assert!(edit_codex("notify = 'invalid'", exe(), 47135, true).is_err());
        assert_eq!(edit_claude("", exe(), 47135, false).unwrap(), "");
        assert_eq!(edit_codex("", exe(), 47135, false).unwrap(), "");
    }
    #[test]
    fn path_edits_back_up_and_invalid_second_file_does_not_change_first() {
        let dir = std::env::temp_dir().join(format!("turzx-hooks-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (c, x) = (dir.join("settings.json"), dir.join("config.toml"));
        let original_c = r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"keep"}]}]},"theme":"dark"}"#;
        let original_x = "# keep\nnotify = [ 'helper', 'turn-ended' ]\nmodel = 'test'\n";
        std::fs::write(&c, original_c).unwrap(); std::fs::write(&x, original_x).unwrap();
        edit_paths(&c, &x, exe(), 47135, true).unwrap();
        assert_eq!(read(&dir.join("settings.json.bak")).unwrap(), original_c);
        assert_eq!(read(&dir.join("config.toml.bak")).unwrap(), original_x);
        let after = (read(&c).unwrap(), read(&x).unwrap());
        edit_paths(&c, &x, exe(), 47135, true).unwrap();
        assert_eq!((read(&c).unwrap(), read(&x).unwrap()), after);
        edit_paths(&c, &x, exe(), 47135, false).unwrap();
        assert_eq!(read(&x).unwrap(), original_x);
        let before = read(&c).unwrap(); std::fs::write(&x, "notify = 'bad'").unwrap();
        assert!(edit_paths(&c, &x, exe(), 47135, true).is_err()); assert_eq!(read(&c).unwrap(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
