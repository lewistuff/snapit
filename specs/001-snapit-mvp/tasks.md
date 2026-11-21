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
- [ ] T007 Spike: status item + Carbon hotkey (`src/main.rs`)
- [ ] T009 **Spike gate**: bundle 288 KB, footprint 13 MB, 0 CPU, 0 sockets (research R7)
- [ ] T008 `mac/hotkey.rs` + `mac/status.rs`: register/unregister by action, menu (Preferences…, Open Save Folder, About, Quit), feedback flash, alerts, main menu
- [ ] T010 `mac/capture.rs`: permission, SCShareableContent prefetch, region capture excluding snapit, PNG, pasteboard, file write
- [ ] T011 `mac/overlay.rs`: per-screen non-activating panels, crosshair, drag rectangle, W×H label, Esc, tiny-selection cancel
- [ ] T012 `app.rs`: hotkey → busy guard → permission → overlay → export → feedback
- [ ] 🧑 T013 Checkpoint: ⌘⇧C, drag, paste. Esc cancels. Focus returns to the previous app.

## Phase 4 — US2 Capture + annotate (P2)
- [ ] T014 `mac/editor.rs` + `mac/render.rs`: window, toolbar, aspect-fit canvas, input → Doc, renderer, text field, flatten
- [ ] T015 `app.rs` editor wiring: copy/save/done, close with discard prompt, release on close
- [ ] 🧑 T016 Checkpoint: ⌘⇧X, every tool, undo/redo, move/delete/recolor, renumbering, Done → paste, ⌘S → file

## Phase 5 — US4 First run and Preferences (P2)
- [ ] T017 Permission onboarding at launch and on a hotkey press without permission; deep link
- [ ] T018 `mac/prefs.rs`: four shortcut recorders, folder picker, save-quick checkbox, launch at login
- [ ] T019 `app.rs` prefs handlers: validate, save, re-register, warn on conflict or registration failure
- [ ] 🧑 T020 Checkpoint: grant flow, rebind, change folder, toggles

## Phase 6 — US3 Window capture (P3)
- [ ] T021 Overlay window mode: CGWindowList hit-test, highlight, desktop-independent window capture with no shadow
- [ ] 🧑 T022 Checkpoint: bind quick-window, capture an overlapped window

## Phase 7 — Polish
- [ ] T023 `cargo clippy` clean; `cargo test` 25 passing, including a pixel test of the render/flatten path
- [ ] T024 `make measure` → bundle 488 KB, footprint 14 MB (SC-001 ✅), RSS 71 MB (shared AppKit pages, not the metric), 0 CPU over 30 s, 0 sockets
- [ ] T025 quickstart.md, README.md
- [ ] 🧑 T026 SC-003 / SC-005 / SC-007 timings with real captures
