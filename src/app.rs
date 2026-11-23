//! Orchestration: hotkey → overlay → capture → clipboard/file or editor, plus
//! Preferences and the menu. App state lives in a main-thread RefCell.
//!
//! Rule: never run a modal loop or close a window while the state is
//! borrowed — those can re-enter these handlers. Handlers return an "after"
//! closure for such work, which runs once the borrow is released.

use std::cell::RefCell;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWorkspace};
use objc2_foundation::NSURL;

use crate::core::annotate::{Doc, Point, Tool};
use crate::core::files::{self, Stamp};
use crate::core::hotkey::Binding;
use crate::core::settings::{self, Action, Settings};
use crate::mac::capture::{self, Image};
use crate::mac::editor::{self, Editor};
use crate::mac::overlay::{self, Mode, Outcome};
use crate::mac::prefs::{self, Prefs};
use crate::mac::status::{self, Status};
use crate::mac::{self, alert, alert_later, beep, hotkey, ns};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Busy {
    Idle,
    Overlay,
    Editor,
}

struct EditorSession {
    doc: Doc,
    image: Image,
    ui: Editor,
}

pub struct App {
    mtm: MainThreadMarker,
    pub status: Status,
    settings: Settings,
    busy: Busy,
    editor: Option<EditorSession>,
    prefs: Option<Prefs>,
    hotkey_errors: [Option<String>; 4],
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

type After = Option<Box<dyn FnOnce()>>;

/// Runs `f` with the app state. Returns None if the state is already borrowed
/// (a re-entrant event, which is dropped) or not yet initialised.
pub fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => guard.as_mut().map(f),
        Err(_) => None,
    })
}

fn run(after: Option<After>) {
    if let Some(Some(f)) = after {
        f();
    }
}

pub fn start(mtm: MainThreadMarker) {
    let (settings, load_err) = settings::load(&settings::path());
    let mut app = App {
        mtm,
        status: Status::new(mtm),
        settings,
        busy: Busy::Idle,
        editor: None,
        prefs: None,
        hotkey_errors: Default::default(),
    };
    hotkey::install(on_hotkey);
    let failures = app.register_hotkeys();
    APP.set(Some(app));

    if let Some(e) = load_err {
        alert_later("Couldn't read snapit settings", &format!("{e}\n\nUsing the defaults."));
    }
    if !failures.is_empty() {
        alert_later("Some shortcuts aren't available", &failures.join("\n\n"));
    }
    // SNAPIT_NO_ONBOARDING lets scripts/measure.sh record true idle memory.
    if !capture::has_permission() && std::env::var_os("SNAPIT_NO_ONBOARDING").is_none() {
        mac::on_main(|mtm| onboarding(mtm, false));
    }
}

impl App {
    /// (Re)registers all hotkeys from settings; returns user-facing failures (FR-002a).
    fn register_hotkeys(&mut self) -> Vec<String> {
        hotkey::unregister_all();
        let mut failures = Vec::new();
        for action in Action::ALL {
            let i = action.index();
            self.hotkey_errors[i] = None;
            let b = match Binding::parse(self.settings.hotkeys.get(action)) {
                Ok(b) if b.is_unbound() => continue,
                Ok(b) => b,
                Err(e) => {
                    self.hotkey_errors[i] = Some(e.clone());
                    failures.push(format!("{}: {e}", action.label()));
                    continue;
                }
            };
            if let Err(status) = hotkey::register(i as u32 + 1, b.code, b.mods) {
                let msg = if status == hotkey::ERR_EXISTS {
                    "Another app already uses this shortcut".to_string()
                } else {
                    format!("macOS refused this shortcut (error {status})")
                };
                failures.push(format!("{} ({}): {msg}. Choose another in Preferences.", b.display(), action.label()));
                self.hotkey_errors[i] = Some(msg);
            }
        }
        let tip: Vec<String> = Action::ALL
            .into_iter()
            .filter(|&a| self.hotkey_errors[a.index()].is_none())
            .filter_map(|a| Binding::parse(self.settings.hotkeys.get(a)).ok().filter(|b| !b.is_unbound()).map(|b| format!("{}  {}", b.display(), a.label())))
            .collect();
        self.status.set_tooltip(&format!("snapit\n{}", tip.join("\n")));
        failures
    }

    fn save_settings(&self) {
        if let Err(e) = settings::save(&settings::path(), &self.settings) {
            alert_later("Couldn't save settings", &e.to_string());
        }
    }

    fn refresh_editor(&self) {
        if let Some(es) = &self.editor {
            let d = &es.doc;
            es.ui.update(d.render(), d.tool, d.color, d.size);
        }
    }

    fn row_warning(&self, action: Action) -> String {
        if let Some(e) = &self.hotkey_errors[action.index()] {
            return e.clone();
        }
        Binding::parse(self.settings.hotkeys.get(action))
            .ok()
            .and_then(|b| b.conflict())
            .map(|c| format!("Overrides {c}"))
            .unwrap_or_default()
    }

    fn refresh_prefs_row(&self, action: Action) {
        if let Some(p) = &self.prefs {
            let display = Binding::parse(self.settings.hotkeys.get(action)).map(|b| b.display()).unwrap_or_default();
            p.set_row(action, &display, &self.row_warning(action));
        }
    }
}

// ---- capture -------------------------------------------------------------

enum Hot {
    Ignore,
    NeedPermission,
    Go,
}

fn on_hotkey(id: u32) {
    let Some(&action) = Action::ALL.get((id as usize).wrapping_sub(1)) else { return };
    let r = with(|a| {
        if a.busy != Busy::Idle {
            return (Hot::Ignore, a.mtm);
        }
        if !capture::has_permission() {
            return (Hot::NeedPermission, a.mtm);
        }
        a.busy = Busy::Overlay;
        (Hot::Go, a.mtm)
    });
    match r {
        None | Some((Hot::Ignore, _)) => beep(),
        Some((Hot::NeedPermission, mtm)) => onboarding(mtm, true),
        Some((Hot::Go, mtm)) => {
            let mode = if action.window() { Mode::Window } else { Mode::Region };
            overlay::begin(mtm, mode, Box::new(move |o| on_overlay(action, o)));
        }
    }
}

fn on_overlay(action: Action, outcome: Outcome) {
    match outcome {
        Outcome::Cancelled => {
            with(|a| a.busy = Busy::Idle);
        }
        Outcome::Captured(Err(e)) => {
            with(|a| a.busy = Busy::Idle);
            let hint = if capture::has_permission() {
                ""
            } else {
                "\n\nsnapit no longer has Screen Recording access. Turn it on in System Settings → Privacy & Security."
            };
            alert_later("Capture failed", &format!("{e}{hint}"));
        }
        Outcome::Captured(Ok((img, scale))) if action.edit() => open_editor(img, scale),
        Outcome::Captured(Ok((img, _))) => quick_export(img),
    }
}

/// A save path for a new capture, or the reason there isn't one.
fn new_path(dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    files::ensure_dir(dir)
        .and_then(|_| files::next_path(dir, Stamp::now()))
        .map_err(|e| format!("Couldn't save to {}: {e}", dir.display()))
}

fn quick_export(img: Image) {
    let Some(save) = with(|a| {
        a.busy = Busy::Idle;
        a.settings.save_quick_captures.then(|| new_path(&a.settings.save_dir))
    }) else {
        return;
    };
    let (path, path_err) = match save {
        Some(Ok(p)) => (Some(p), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    capture::export(img, true, path, move |res| {
        if let Some(e) = path_err.or(res.err()) {
            alert_later("Copied to the clipboard, but not saved", &e);
        }
    });
    with(|a| a.status.flash(&a.settings.sound));
}

// ---- editor --------------------------------------------------------------

fn open_editor(img: Image, scale: f64) {
    let Some(mtm) = with(|a| a.mtm) else { return };
    let size = capture::size(&img);
    let ui = editor::open(mtm, &img, scale);
    let doc = Doc::new(size.width, size.height, scale, Some(Box::new(editor::measure)));
    with(|a| {
        a.editor = Some(EditorSession { doc, image: img, ui });
        a.busy = Busy::Editor;
        a.refresh_editor();
    });
}

#[derive(Debug)]
pub enum EditorEvent {
    Press(Point, usize),
    Drag(Point),
    Release(Point),
    Tool(Tool),
    Color(usize),
    Size(usize),
    Undo,
    Redo,
    Delete,
    Deselect,
    TextCommit(u32, Point, String),
    TextCancel,
    Copy,
    Save,
    Done,
}

enum Export {
    Copy { close: bool },
    Save,
}

pub fn editor_event(ev: EditorEvent) {
    let after = with(|a| -> After {
        let es = a.editor.as_mut()?;
        let d = &mut es.doc;
        let mut export = None;
        match ev {
            EditorEvent::Press(p, clicks) => {
                let eff = d.press(p, clicks);
                if eff.begin_text {
                    let ui = es.ui.clone();
                    let (fs, color) = (d.font_size(d.size), d.color);
                    let (fs, color) = d
                        .annots
                        .iter()
                        .find(|x| x.id == eff.text_id)
                        .map_or((fs, color), |x| (d.font_size(x.size), x.color));
                    a.refresh_editor();
                    return Some(Box::new(move || ui.begin_text(&eff, fs, color)));
                }
            }
            EditorEvent::Drag(p) => d.drag_to(p),
            EditorEvent::Release(p) => d.release(p),
            EditorEvent::Tool(t) => d.set_tool(t),
            EditorEvent::Color(c) => d.set_color(c),
            EditorEvent::Size(s) => d.set_size(s),
            EditorEvent::Undo => d.undo(),
            EditorEvent::Redo => d.redo(),
            EditorEvent::Delete => d.delete_selected(),
            EditorEvent::Deselect => d.deselect(),
            EditorEvent::TextCommit(id, at, text) => d.commit_text(id, at, &text),
            EditorEvent::TextCancel => d.cancel_text(),
            EditorEvent::Copy => export = Some(Export::Copy { close: false }),
            EditorEvent::Done => export = Some(Export::Copy { close: true }),
            EditorEvent::Save => export = Some(Export::Save),
        }
        a.refresh_editor();
        let export = export?;

        let es = a.editor.as_mut()?;
        let mut items = es.doc.render();
        items.iter_mut().for_each(|i| i.selected = false);
        let Some(flat) = editor::flatten(&es.image, &items) else {
            alert_later("Couldn't export the image", "Drawing the annotations failed.");
            return None;
        };
        match export {
            Export::Copy { close } => {
                es.doc.mark_clean();
                capture::export(flat, true, None, |_| {});
                a.status.flash(&a.settings.sound);
                close.then(|| {
                    let ui = es.ui.clone();
                    Box::new(move || ui.close()) as Box<dyn FnOnce()>
                })
            }
            Export::Save => {
                match new_path(&a.settings.save_dir) {
                    Ok(path) => capture::export(flat, false, Some(path), |res| match res {
                        Ok(()) => {
                            with(|a| {
                                a.status.flash(&a.settings.sound);
                                if let Some(es) = &mut a.editor {
                                    es.doc.mark_clean();
                                }
                            });
                        }
                        Err(e) => alert_later("Couldn't save", &e),
                    }),
                    Err(e) => alert_later("Couldn't save", &e),
                }
                None
            }
        }
    });
    run(after);
}

/// Asked by the window before closing; prompts if there are unexported edits.
pub fn editor_should_close() -> bool {
    let Some((dirty, mtm)) = with(|a| (a.editor.as_ref().is_some_and(|e| e.doc.dirty()), a.mtm)) else {
        return true;
    };
    !dirty
        || alert(
            mtm,
            "Discard your annotations?",
            "They haven't been copied or saved.",
            &["Discard", "Cancel"],
            true,
        ) == 0
}

pub fn editor_closed() {
    let hide = with(|a| {
        a.editor = None;
        a.busy = Busy::Idle;
        a.prefs.is_none().then_some(a.mtm)
    });
    // Hand focus back to the app the user was in.
    if let Some(Some(mtm)) = hide {
        NSApplication::sharedApplication(mtm).hide(None);
    }
}

// ---- menu & preferences ----------------------------------------------------

pub enum MenuCommand {
    Preferences,
    OpenFolder,
    About,
}

pub fn on_menu(cmd: MenuCommand) {
    let after = with(|a| -> After {
        let mtm = a.mtm;
        match cmd {
            MenuCommand::Preferences => {
                mac::activate(mtm);
                if let Some(p) = &a.prefs {
                    p.front();
                } else {
                    let p = prefs::open(mtm, &a.settings, prefs::login_item_enabled());
                    a.prefs = Some(p);
                    Action::ALL.into_iter().for_each(|act| a.refresh_prefs_row(act));
                }
                None
            }
            MenuCommand::OpenFolder => {
                let dir = a.settings.save_dir.clone();
                if let Err(e) = files::ensure_dir(&dir) {
                    alert_later("Couldn't open the save folder", &e.to_string());
                    return None;
                }
                let url = NSURL::fileURLWithPath(&ns(&dir.to_string_lossy()));
                NSWorkspace::sharedWorkspace().openURL(&url);
                None
            }
            MenuCommand::About => {
                mac::activate(mtm);
                let app = NSApplication::sharedApplication(mtm);
                app.orderFrontStandardAboutPanel(None);
                None
            }
        }
    });
    run(after);
}

#[derive(Debug)]
pub enum PrefsEvent {
    /// A shortcut recorder is listening: hotkeys must not swallow its keys.
    Recording(bool),
    SetHotkey(Action, Binding),
    SetFolder(std::path::PathBuf),
    SaveQuick(bool),
    LaunchAtLogin(bool),
    /// A system sound name, or empty for none.
    Sound(String),
    Closed,
}

pub fn prefs_event(ev: PrefsEvent) {
    with(|a| match ev {
        PrefsEvent::Recording(true) => hotkey::unregister_all(),
        PrefsEvent::Recording(false) => {
            a.register_hotkeys();
        }
        PrefsEvent::SetHotkey(action, b) => {
            if let Err(e) = b.validate() {
                a.register_hotkeys();
                if let Some(p) = &a.prefs {
                    let cur = Binding::parse(a.settings.hotkeys.get(action)).map(|b| b.display()).unwrap_or_default();
                    p.set_row(action, &cur, &e);
                }
                return;
            }
            if !b.is_unbound()
                && let Some(other) = Action::ALL
                    .into_iter()
                    .find(|&o| o != action && Binding::parse(a.settings.hotkeys.get(o)).ok() == Some(b))
            {
                a.register_hotkeys();
                if let Some(p) = &a.prefs {
                    let cur = Binding::parse(a.settings.hotkeys.get(action)).map(|b| b.display()).unwrap_or_default();
                    p.set_row(action, &cur, &format!("{} is already used for “{}”", b.display(), other.label()));
                }
                return;
            }
            a.settings.hotkeys.set(action, b.to_string());
            a.save_settings();
            a.register_hotkeys();
            Action::ALL.into_iter().for_each(|act| a.refresh_prefs_row(act));
        }
        PrefsEvent::SetFolder(dir) => {
            a.settings.save_dir = dir;
            a.save_settings();
            if let Some(p) = &a.prefs {
                p.set_folder(&a.settings.save_dir);
            }
        }
        PrefsEvent::SaveQuick(on) => {
            a.settings.save_quick_captures = on;
            a.save_settings();
        }
        PrefsEvent::Sound(name) => {
            status::play_sound(&name);
            a.settings.sound = name;
            a.save_settings();
        }
        PrefsEvent::LaunchAtLogin(on) => match prefs::set_login_item(on) {
            Ok(()) => {
                a.settings.launch_at_login = on;
                a.save_settings();
            }
            Err(e) => {
                if let Some(p) = &a.prefs {
                    p.set_login(prefs::login_item_enabled());
                }
                alert_later("Couldn't change the login item", &e);
            }
        },
        PrefsEvent::Closed => {
            a.prefs = None;
            a.register_hotkeys();
        }
    });
}

// ---- permission ----------------------------------------------------------

/// Explains Screen Recording access and offers to open System Settings
/// (Story 4). `from_hotkey` is set when a capture was attempted without it.
fn onboarding(mtm: MainThreadMarker, from_hotkey: bool) {
    let title = if from_hotkey {
        "snapit can't capture the screen yet"
    } else {
        "snapit needs Screen Recording access"
    };
    let choice = alert(
        mtm,
        title,
        "To take screenshots, macOS needs you to allow snapit under System Settings → Privacy & Security → Screen Recording.\n\nAfter turning it on, quit and reopen snapit.",
        &["Open System Settings", "Later"],
        false,
    );
    if choice == 0 {
        capture::request_permission();
        capture::open_privacy_settings(mtm);
    }
}
