# Tasks: snapit MVP

`[P]` = can run in parallel (different files, no dependency). `[US#]` = user story. 🧑 = checkpoint that a person must verify by hand: it needs the permission prompt, real hotkeys or mouse drags, which an agent can't do.

## Phase 1 — Setup
- [x] T001 Cargo crate `snapit`, size-optimised release profile, `.gitignore`
- [x] T002 `build/Info.plist`, `Makefile` (test/build/app/run/measure), `scripts/measure.sh`

## Phase 2 — Foundation (pure Rust, `src/core`, tested)
- [x] T003 [P] `settings`: defaults, load/save, partial and corrupt files
- [x] T004 [P] `files`: `snapit-YYYY-MM-DD-HHMMSS.png`, collision suffixes, `ensure_dir`, local time
- [x] T005 [P] `hotkey`: parse/format, Carbon keycodes, modifier masks, conflict list
- [x] T006 [P] `annotate`: Doc, tool state machine, hit-testing, steps, undo/redo, dirty flag

## Phase 3 — US1 Quick region capture (P1) 🎯
- [x] T007 Spike: status item + Carbon hotkey (`src/main.rs`)
- [x] T009 **Spike gate**: bundle 288 KB, footprint 13 MB, 0 CPU, 0 sockets (research R7)
- [x] T008 `mac/hotkey.rs` + `mac/status.rs`: register/unregister by action, menu (Preferences…, Open Save Folder, About, Quit), feedback flash, alerts, main menu
- [x] T010 `mac/capture.rs`: permission, SCShareableContent prefetch, region capture excluding snapit, PNG, pasteboard, file write
- [x] T011 `mac/overlay.rs`: per-screen non-activating panels, crosshair, drag rectangle, W×H label, Esc, tiny-selection cancel
- [x] T012 `app.rs`: hotkey → busy guard → permission → overlay → export → feedback
- [ ] 🧑 T013 Checkpoint: ⌘⇧C, drag, paste. Esc cancels. Focus returns to the previous app.

## Phase 4 — US2 Capture + annotate (P2)
- [x] T014 `mac/editor.rs` + `mac/render.rs`: window, toolbar, aspect-fit canvas, input → Doc, renderer, text field, flatten
- [x] T015 `app.rs` editor wiring: copy/save/done, close with discard prompt, release on close
- [ ] 🧑 T016 Checkpoint: ⌘⇧X, every tool, undo/redo, move/delete/recolor, renumbering, Done → paste, ⌘S → file

## Phase 5 — US4 First run and Preferences (P2)
- [x] T017 Permission onboarding at launch and on a hotkey press without permission; deep link
- [x] T018 `mac/prefs.rs`: four shortcut recorders, folder picker, save-quick checkbox, launch at login
- [x] T019 `app.rs` prefs handlers: validate, save, re-register, warn on conflict or registration failure
- [ ] 🧑 T020 Checkpoint: grant flow, rebind, change folder, toggles

## Phase 6 — US3 Window capture (P3)
- [x] T021 Overlay window mode: CGWindowList hit-test, highlight, desktop-independent window capture with no shadow
- [ ] 🧑 T022 Checkpoint: bind quick-window, capture an overlapped window

## Phase 7 — Polish
- [x] T023 `cargo clippy` clean; `cargo test` 25 passing, including a pixel test of the render/flatten path
- [x] T024 `make measure` → bundle 488 KB, footprint 14 MB (SC-001 ✅), RSS 71 MB (shared AppKit pages, not the metric), 0 CPU over 30 s, 0 sockets
- [x] T025 quickstart.md, README.md
- [ ] 🧑 T026 SC-003 / SC-005 / SC-007 timings with real captures

## Phase 8 — Feedback round 1 (2026-11-21)
- [x] T027 Editor: native unified `NSToolbar`. The window grows until the toolbar reports every item visible, so nothing goes into the overflow menu.
- [x] T028 Default annotation size is small. Sizes are shown as dots. Keys: 1–6 for colors, [ and ] for size. Every toolbar tooltip names its shortcut, and the menu bar tooltip lists the capture hotkeys.
- [x] T029 Stable signing: `scripts/sign.sh` with a self-signed `snapit-dev` certificate in its own keychain.
- [x] T030 Aperture app icon (`scripts/make-icon.swift` → `assets/AppIcon.icns`) and a matching template menu bar icon.

## Phase 9 — Benchmarks (2026-11-23)
- [x] T031 `make bench`: `#[ignore]`d micro-benchmarks in `src/bench.rs` (PNG encode at Retina/5K for UI and worst-case content, pasteboard write, flatten, Doc render/hit-test/drag/undo). No new dependencies. Results in README.md
