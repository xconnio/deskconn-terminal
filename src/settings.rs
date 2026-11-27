//! The user's settings, kept in `settings.json` in the app's configuration directory
//! (`~/.config/deskconn-terminal` on Linux).

use std::path::PathBuf;

use anyhow::{Context, Result};
use gpui::{App, Global};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The terminals' font size, in pixels.
    pub font_size: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { font_size: 14. }
    }
}

impl Global for Settings {}

impl Settings {
    /// The smallest and largest font sizes there can be.
    pub const FONT_SIZES: (f32, f32) = (8., 40.);

    /// The saved settings, or the defaults if there are none or they can't be read.
    pub fn load() -> Self {
        let Some(path) = path() else {
            return Self::default();
        };
        let mut settings = match std::fs::read_to_string(&path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_else(|err| {
                eprintln!("ignoring {}: {err}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        };
        let (min, max) = Self::FONT_SIZES;
        settings.font_size = settings.font_size.clamp(min, max);
        settings
    }

    /// Saves the settings, replacing the file whole so that it's never left half written.
    pub fn save(&self) -> Result<()> {
        let path = path().context("no configuration directory")?;
        let dir = path.parent().expect("the settings file is in a directory");
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_string_pretty(self)? + "\n")
            .with_context(|| format!("write {}", temporary.display()))?;
        std::fs::rename(&temporary, &path).with_context(|| format!("write {}", path.display()))
    }

    pub fn get(cx: &App) -> &Self {
        cx.global()
    }

    /// Changes the settings, saves them, and redraws the windows with them.
    pub fn update(cx: &mut App, change: impl FnOnce(&mut Self)) {
        let settings = cx.global_mut::<Self>();
        change(settings);
        if let Err(err) = settings.save() {
            eprintln!("save the settings: {err:#}");
        }
        cx.refresh_windows();
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("deskconn-terminal").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_are_defaults() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, Settings::default());
    }
}
