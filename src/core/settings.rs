//! Persists preferences as JSON in ~/Library/Application Support/snapit/settings.json.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::files;

/// One of the four capture actions that can be bound to a hotkey.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    QuickRegion,
    EditRegion,
    QuickWindow,
    EditWindow,
}

impl Action {
    pub const ALL: [Action; 4] =
        [Action::QuickRegion, Action::EditRegion, Action::QuickWindow, Action::EditWindow];

    pub fn label(self) -> &'static str {
        match self {
            Action::QuickRegion => "Capture region",
            Action::EditRegion => "Capture region & annotate",
            Action::QuickWindow => "Capture window",
            Action::EditWindow => "Capture window & annotate",
        }
    }

    /// Whether the action opens the editor.
    pub fn edit(self) -> bool {
        matches!(self, Action::EditRegion | Action::EditWindow)
    }

    /// Whether the action captures a window rather than a region.
    pub fn window(self) -> bool {
        matches!(self, Action::QuickWindow | Action::EditWindow)
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub quick_region: String,
    pub edit_region: String,
    pub quick_window: String,
    pub edit_window: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Hotkeys {
            quick_region: "shift+cmd+c".into(),
            edit_region: "shift+cmd+x".into(),
            quick_window: String::new(),
            edit_window: String::new(),
        }
    }
}

impl Hotkeys {
    fn field(&mut self, a: Action) -> &mut String {
        match a {
            Action::QuickRegion => &mut self.quick_region,
            Action::EditRegion => &mut self.edit_region,
            Action::QuickWindow => &mut self.quick_window,
            Action::EditWindow => &mut self.edit_window,
        }
    }

    pub fn get(&self, a: Action) -> &str {
        match a {
            Action::QuickRegion => &self.quick_region,
            Action::EditRegion => &self.edit_region,
            Action::QuickWindow => &self.quick_window,
            Action::EditWindow => &self.edit_window,
        }
    }

    pub fn set(&mut self, a: Action, s: impl Into<String>) {
        *self.field(a) = s.into();
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hotkeys: Hotkeys,
    pub save_dir: PathBuf,
    pub save_quick_captures: bool,
    pub launch_at_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hotkeys: Hotkeys::default(),
            save_dir: files::default_dir(),
            save_quick_captures: false,
            launch_at_login: false,
        }
    }
}

/// The settings file location.
pub fn path() -> PathBuf {
    files::home().join("Library/Application Support/snapit/settings.json")
}

/// Reads settings from path. A missing file yields defaults and no error; an
/// unreadable or corrupt file yields defaults and the error, so the app can
/// still start. Fields absent from the file keep their defaults.
pub fn load(path: &Path) -> (Settings, Option<io::Error>) {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (Settings::default(), None),
        Err(e) => return (Settings::default(), Some(e)),
    };
    match serde_json::from_slice::<Settings>(&data) {
        Ok(mut s) => {
            if s.save_dir.as_os_str().is_empty() {
                s.save_dir = files::default_dir();
            }
            s.save_dir = files::expand_home(&s.save_dir.to_string_lossy());
            (s, None)
        }
        Err(e) => (Settings::default(), Some(e.into())),
    }
}

/// Writes settings atomically.
pub fn save(path: &Path, s: &Settings) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut data = serde_json::to_vec_pretty(s)?;
    data.push(b'\n');
    let tmp = dir.join(format!(".settings-{}.json", std::process::id()));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("snapit-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn defaults() {
        let d = Settings::default();
        assert_eq!(d.hotkeys.get(Action::QuickRegion), "shift+cmd+c");
        assert_eq!(d.hotkeys.get(Action::EditRegion), "shift+cmd+x");
        assert_eq!(d.hotkeys.get(Action::QuickWindow), "");
        assert_eq!(d.hotkeys.get(Action::EditWindow), "");
        assert!(!d.save_quick_captures && !d.launch_at_login);
        assert_eq!(d.save_dir, files::default_dir());
    }

    #[test]
    fn load_missing() {
        let (s, err) = load(&tmp("missing").join("nope.json"));
        assert!(err.is_none());
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn round_trip() {
        let p = tmp("rt").join("snapit/settings.json");
        let mut s = Settings::default();
        s.hotkeys.set(Action::EditRegion, "");
        s.hotkeys.set(Action::QuickWindow, "opt+cmd+w");
        s.save_dir = "/tmp/shots".into();
        s.save_quick_captures = true;
        save(&p, &s).unwrap();
        let (got, err) = load(&p);
        assert!(err.is_none());
        assert_eq!(got, s);
    }

    #[test]
    fn partial_keeps_defaults() {
        let p = tmp("partial").join("s.json");
        std::fs::write(&p, r#"{"save_quick_captures": true}"#).unwrap();
        let (s, err) = load(&p);
        assert!(err.is_none());
        assert!(s.save_quick_captures);
        assert_eq!(s.hotkeys.get(Action::QuickRegion), "shift+cmd+c");
        assert_eq!(s.save_dir, files::default_dir());
    }

    #[test]
    fn corrupt() {
        let p = tmp("corrupt").join("s.json");
        std::fs::write(&p, "{nope").unwrap();
        let (s, err) = load(&p);
        assert!(err.is_some());
        assert_eq!(s, Settings::default());
    }
}
