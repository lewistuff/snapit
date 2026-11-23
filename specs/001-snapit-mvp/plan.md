# Implementation Plan: snapit MVP

**Branch**: `001-snapit-mvp` | **Date**: 2026-11-21 (revised: Go → Rust) | **Spec**: [spec.md](./spec.md) | **Research**: [research.md](./research.md)

## Summary

snapit is a menu bar–only macOS app written in Rust. A pure-Rust core (settings, hotkey parsing, file naming, the annotation model with undo) drives AppKit, ScreenCaptureKit and Carbon hotkeys, called directly through the `objc2` bindings. There is no second language and no FFI bridge of our own. Pixels stay in `CGImage`s. Every window is created when needed and destroyed when closed. The spike measured a 288 KB bundle and a 13 MB footprint (R7).

## Technical Context

| | |
|---|---|
| Language | Rust 1.100, edition 2024 |
| Dependencies | `objc2` family (framework bindings), `block2`, `dispatch2`, `serde` + `serde_json`. Nothing else. |
| Frameworks | AppKit, ScreenCaptureKit, CoreGraphics, ImageIO, ServiceManagement, Carbon (HIToolbox hotkeys only) |
| Storage | `~/Library/Application Support/snapit/settings.json`. Images go to `~/Pictures/snapit/` |
| Testing | `cargo test` covers `src/core`. Manual checkpoints for UI, permission and capture (see tasks.md) |
| Target | macOS 14.0+, arm64 (amd64 builds with `--target x86_64-apple-darwin`) |
| Performance | phys_footprint < 20 MB idle (SC-001, R7), bundle < 10 MB, hotkey → overlay < 100 ms, quick capture < 300 ms, idle CPU ≈ 0 |
| Release profile | `opt-level="z"`, fat LTO, `codegen-units=1`, `panic="abort"`, `strip` |

## Architecture

```
src/main.rs            entry: MainThreadMarker, NSApplication (accessory), run loop
src/core/              pure Rust, no macOS types, unit-tested
    settings.rs        load/save JSON (serde, partial files keep defaults)
    files.rs           timestamped names, collision suffixes, ensure dir
    hotkey.rs          "shift+cmd+x" ⇄ Carbon keycode/mods, "⇧⌘X", conflict list
    annotate.rs        annotation Doc, tool state machine, hit-testing, steps, undo/redo
src/app.rs             orchestration: hotkey → busy guard → permission → overlay → export/editor
src/mac/               AppKit / ScreenCaptureKit glue, one file per concern
    hotkey.rs          Carbon RegisterEventHotKey through a hand-written extern "C" block (R2)
    status.rs          status item + menu, feedback flash, alerts, main menu (key equivalents)
    overlay.rs         per-screen non-activating NSPanel: region drag / window hover-highlight
    capture.rs         permission, SCShareableContent, screenshot, PNG encode, pasteboard, file write
    editor.rs          editor window, toolbar, canvas NSView, text field
    render.rs          draws annotate::Item lists into a CGContext (canvas and export)
    prefs.rs           Preferences window, shortcut recorder, folder picker, SMAppService
build/Info.plist       LSUIElement, LSMinimumSystemVersion 14.0
Makefile               test / build / app / run / measure
scripts/measure.sh     bundle size, RSS + phys_footprint, CPU, sockets
```

### Native layer

- **Threading**: everything runs on the main thread. `MainThreadMarker` is threaded through every UI call. ScreenCaptureKit completions and PNG encoding run on background queues and hop back with `dispatch2` `DispatchQueue::main().exec_async` before touching the UI or the pasteboard.
- **State**: one `App` struct lives in a main-thread `thread_local!` `RefCell`. Objective-C callbacks (Carbon hotkey, `define_class!` action methods, view events) call `app::with(|app| …)`. While a borrow is held, nothing calls back into code that borrows again: the editor drawing code reads only the render list, which is cloned before `setNeedsDisplay`.
- **Split of work**: `core::annotate::Doc` decides *what* happens. `mac::editor` passes on input and asks `mac::render` to draw `doc.render()`. The canvas and the exported image use the same render function, so the export matches the screen exactly.

### Flows

1. **Quick capture (⌘⇧C)**:
   1. The Carbon handler calls `app::on_hotkey(action)`.
   2. The app checks it isn't busy and has permission, then calls `overlay::begin(Region)`.
   3. The overlay prefetches `SCShareableContent`. On mouse release it captures with snapit excluded.
   4. `app::on_captured(img)` calls `capture::export(img, clipboard, save_path)`.
   5. A promise of the PNG goes on the pasteboard at once and the menu bar icon flashes and a sound plays.
   6. The PNG is encoded off the main thread and the file written if needed. A paste before then waits for the encode; afterwards the promise is swapped for the real data on main.
2. **Capture + edit (⌘⇧X)**: same steps up to `on_captured`. Then `editor::open(img, scale)`.
   - Input goes to `Doc` and then a redraw.
   - Copy, Save or Done flattens the image (`render` into a bitmap context) and then takes the export path above.
3. **Preferences**: controls write to `Settings`, which is saved, and hotkeys are re-registered. While a shortcut is being recorded, all hotkeys are unregistered so the recorder can see them.

### Key decisions

| Decision | Why | Rejected |
|---|---|---|
| Rust + `objc2` | One language, no hand-written bridge, main-thread rules checked at compile time, tiny binary (R7) | Go + cgo + Objective-C (original plan), Swift (user asked for Rust) |
| Overlay is a **non-activating** `NSPanel` at screen-saver level, one per screen, `CanJoinAllSpaces \| FullScreenAuxiliary` | Esc works and the previous app keeps focus, and the overlay shows over full-screen apps | Activating the app and restoring focus afterwards |
| Exclude snapit from the capture filter | No race against the window server | Hiding the overlay, then waiting |
| Undo by snapshotting the annotation list | Small documents; trivially correct | Command pattern |
| Step numbers derived from order | Renumbering after delete (FR-016) is automatic | Stored numbers |
| Pixels only in `CGImage`/ImageIO | Memory returns to idle quickly (SC-005) | `image` crate |

### Quality gates

1. Only binding crates plus serde as dependencies.
2. `cargo clippy` and `cargo test` pass.
3. `make measure` numbers are recorded against SC-001, SC-002 and SC-004.

## Risks

- **Carbon symbols removed in a future macOS** (R2): isolated in `mac/hotkey.rs`. The fallback is an `NSEvent` monitor plus an Accessibility prompt.
- **Re-granting Screen Recording after each ad-hoc rebuild** (R4): see quickstart.md.
