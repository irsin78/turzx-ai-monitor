//! Format-preserving edits of config.toml using the TOML parser's byte spans: only the values
//! that changed are replaced; comments, unknown keys, quoting, CRLFs and table order are kept.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use toml::de::{DeTable, DeValue};

use crate::config::{self, Config};

pub struct Document {
    pub path: PathBuf,
    original: Option<String>,
    pub config: Config,
}

impl Document {
    pub fn load(path: PathBuf) -> Result<Self> {
        let original = read_optional(&path)?;
        let config = match &original {
            Some(text) => toml::from_str(text).context("config.toml is not valid; fix it in a text editor first")?,
            None => Config::default(),
        };
        Ok(Self { path, original, config })
    }

    pub fn save(&mut self, edited: &Config) -> Result<()> {
        validate(edited)?;
        ensure!(read_optional(&self.path)? == self.original, "The file changed while the settings were open. Open the settings again to load it.");
        let before = self.original.clone().unwrap_or_else(|| config::template(&self.config));
        let after = update(&before, &self.config, edited)?;
        if self.original.as_deref() != Some(&after) {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // replace atomically: the dashboard never reads a half-written file
            let temp = self.path.with_extension(format!("settings-{}.tmp", std::process::id()));
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
            let result = (|| -> Result<()> {
                use std::io::Write;
                use windows::core::HSTRING;
                use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
                file.write_all(after.as_bytes())?;
                file.sync_all()?;
                drop(file);
                unsafe { MoveFileExW(&HSTRING::from(temp.as_os_str()), &HSTRING::from(self.path.as_os_str()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) }?;
                Ok(())
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&temp);
            }
            result.context("could not save config.toml")?;
        }
        self.original = Some(after);
        self.config = edited.clone();
        Ok(())
    }

    /// Open the file in Notepad (written with the defaults first if there is none).
    pub fn edit_file(&self) -> Result<()> {
        if self.original.is_none() && !self.path.exists() {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&self.path, config::template(&self.config))?;
        }
        std::process::Command::new("notepad.exe").arg(&self.path).spawn()?;
        Ok(())
    }
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

pub fn validate(c: &Config) -> Result<()> {
    ensure!(c.language == "auto" || crate::i18n::LANGUAGES.iter().any(|s| s.code == c.language), "Select a supported language.");
    ensure!(["12h", "24h"].contains(&c.clock.as_str()), "The clock must be 12h or 24h.");
    ensure!(["C", "F"].iter().any(|u| c.temperature.eq_ignore_ascii_case(u)), "The temperature unit must be C or F.");
    ensure!([0, 180].contains(&c.rotation), "The rotation must be 0 or 180.");
    ensure!(
        (!c.location.city.trim().is_empty() || (c.location.latitude.is_some() && c.location.longitude.is_some()))
            && c.location.city.chars().count() <= 120
            && !c.location.city.chars().any(char::is_control),
        "Enter a city (1-120 characters)."
    );
    ensure!(["auto", "kr", "us"].contains(&c.air_quality.scale.as_str()), "Select an air-quality scale.");
    ensure!(c.offline.tmdb_key.chars().all(|ch| ch.is_ascii_graphic()) && c.offline.tmdb_key.len() <= 400, "The TMDB key has unexpected characters.");
    Ok(())
}

/// `text` with the values the form can change set from `new` (where they differ from `old`).
fn update(text: &str, old: &Config, new: &Config) -> Result<String> {
    validate(new)?;
    let mut result = text.to_owned();
    macro_rules! change {
        ($path:expr, $old:expr, $new:expr) => {
            if $old != $new {
                result = replace(&result, $path, toml::Value::try_from($new)?.to_string())?;
            }
        };
    }
    change!(&["language"], &old.language, &new.language);
    change!(&["clock"], &old.clock, &new.clock);
    change!(&["temperature"], &old.temperature, &new.temperature);
    change!(&["rotation"], old.rotation, new.rotation);
    change!(&["location", "city"], &old.location.city, &new.location.city);
    change!(&["air_quality", "scale"], &old.air_quality.scale, &new.air_quality.scale);
    change!(&["ai", "claude"], old.ai.claude, new.ai.claude);
    change!(&["ai", "codex"], old.ai.codex, new.ai.codex);
    change!(&["ai", "antigravity"], old.ai.antigravity, new.ai.antigravity);
    change!(&["offline", "tmdb_key"], &old.offline.tmdb_key, &new.offline.tmdb_key);
    let _: Config = toml::from_str(&result).context("the edited file is not valid TOML")?;
    Ok(result)
}

/// Set the key at `path` to `value` (TOML source), adding the key or table if missing.
fn replace(text: &str, path: &[&str], value: String) -> Result<String> {
    let root = DeTable::parse(text)?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut table = root.get_ref();
    let mut parent = None;
    for key in &path[..path.len() - 1] {
        match table.iter().find(|(k, _)| k.get_ref().as_ref() == *key) {
            Some((_, item)) => {
                let DeValue::Table(inner) = item.get_ref() else { anyhow::bail!("{key} must be a table") };
                parent = Some(item.span());
                table = inner;
            }
            None => return Ok(format!("{text}{newline}[{}]{newline}{} = {value}{newline}", path[..path.len() - 1].join("."), path.last().unwrap())),
        }
    }
    let key = *path.last().unwrap();
    let mut result = text.to_owned();
    if let Some((_, item)) = table.iter().find(|(k, _)| k.get_ref().as_ref() == key) {
        result.replace_range(item.span(), &value);
    } else if let Some(span) = parent {
        let source = &text[span.clone()];
        if source.starts_with('{') {
            let pos = span.end - 1;
            let comma = if table.is_empty() || text[..pos].trim_end().ends_with(',') { "" } else { "," };
            result.insert_str(pos, &format!("{comma} {key} = {value} "));
        } else if source.starts_with('[') {
            let pos = text[span.end..].find('\n').map_or(text.len(), |n| span.end + n + 1);
            let prefix = if pos > 0 && !text[..pos].ends_with('\n') { newline } else { "" };
            result.insert_str(pos, &format!("{prefix}{key} = {value}{newline}"));
        } else {
            // a table made by dotted keys
            result.insert_str(0, &format!("{} = {value}{newline}", path.join(".")));
        }
    } else {
        result.insert_str(0, &format!("{key} = {value}{newline}"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_comments_order_unknown_keys_and_crlf() {
        let input = "# personal settings\r\nlanguage = 'ko' # keep language comment\r\nrotation = 0 # upright\r\nunknown = [1, 2]\r\n[location] # hometown\r\ncity = 'Seoul' # preserve\r\nlatitude = 37.5\r\n[plan_labels.codex]\r\npro = 'My plan'\r\n[future]\r\ncustom = '''leave\nthis alone'''\r\n";
        let old: Config = toml::from_str(input).unwrap();
        assert_eq!(update(input, &old, &old).unwrap(), input);
        let mut new = old.clone();
        new.rotation = 180;
        new.location.city = "Busan \"Harbor\"".into();
        let output = update(input, &old, &new).unwrap();
        assert_eq!(output, input.replace("rotation = 0", "rotation = 180").replace("'Seoul'", "'Busan \"Harbor\"'"));
        let decoded: Config = toml::from_str(&output).unwrap();
        assert_eq!(decoded.location.city, new.location.city);
        assert_eq!(decoded.location.latitude, Some(37.5));
    }

    #[test]
    fn adds_missing_keys_to_explicit_inline_dotted_and_absent_tables() {
        for input in ["# blank\n", "[ai] # flags\ncustom = 1\n", "ai = { custom = 1 } # compact\n", "ai.custom = 1\n", "[ai]"] {
            let old: Config = toml::from_str(input).unwrap();
            let mut new = old.clone();
            new.ai.claude = false;
            new.ai.codex = false;
            new.language = "ko".into();
            let output = update(input, &old, &new).unwrap();
            let actual: Config = toml::from_str(&output).unwrap();
            assert!(!actual.ai.claude && !actual.ai.codex);
            assert_eq!(actual.language, "ko");
            if input.contains("custom") {
                assert!(output.contains("custom = 1"));
            }
        }
    }

    #[test]
    fn rejects_invalid_inputs_and_does_not_rewrite_bad_toml() {
        let mut c = Config::default();
        c.rotation = 90;
        assert!(validate(&c).is_err());
        c.rotation = 0;
        c.location.city = "  ".into();
        assert!(validate(&c).is_err());
        assert!(replace("broken = [", &["clock"], "\"24h\"".into()).is_err());
    }

    #[test]
    fn all_controls_round_trip_and_leave_advanced_keys_intact() {
        let text = config::template(&Config::default());
        let old: Config = toml::from_str(&text).unwrap();
        let mut new = old.clone();
        new.language = "en".into();
        new.clock = "24h".into();
        new.temperature = "F".into();
        new.rotation = 180;
        new.location.city = "New York".into();
        new.air_quality.scale = "us".into();
        new.ai.claude = false;
        new.ai.codex = false;
        new.ai.antigravity = false;
        let output = update(&text, &old, &new).unwrap();
        let actual: Config = toml::from_str(&output).unwrap();
        assert_eq!(toml::to_string(&actual).unwrap(), toml::to_string(&new).unwrap());
        assert!(output.contains("# codex.pro = \"Pro 200\""));
    }

    #[test]
    fn disk_save_preserves_text_rejects_conflicts_and_cancel_does_not_create_a_file() {
        let base = std::env::temp_dir().join("turzx-settings-test");
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join(format!("document-test-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        // opening and closing without saving (Cancel) writes nothing, also on the first run
        drop(Document::load(path.clone()).unwrap());
        assert!(!path.exists());
        let text = "# retain me\nrotation = 0 # upright\nfuture_key = 'keep'\n";
        std::fs::write(&path, text).unwrap();
        let mut document = Document::load(path.clone()).unwrap();
        let mut c = document.config.clone();
        c.rotation = 180;
        document.save(&c).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text.replace("= 0", "= 180"));
        let external = "# external editor won\nrotation = 0\n";
        std::fs::write(&path, external).unwrap();
        assert!(document.save(&c).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
        std::fs::write(&path, "invalid = [").unwrap();
        assert!(Document::load(path.clone()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "invalid = [");
        std::fs::remove_file(path).unwrap();
    }
}
