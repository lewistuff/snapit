# Feature Specification: snapit MVP — Lightweight macOS Screenshot & Annotation Tool

**Feature Branch**: `001-snapit-mvp`
**Created**: 2026-11-20
**Status**: Draft
**Input**: User description: "A lightweight, open-source macOS menu bar screenshot tool for developers, written in Rust. Region and window capture via global hotkeys, with a quick annotation editor. Optimise for the lightest runtime and memory footprint."

## Clarifications

### Session 2026-11-20

- Q: What does snapit capture? → A: Screenshots.
- Q: What form factor? → A: A desktop menu bar (tray) app.
- Q: Who is the audience? → A: Developers broadly (open source).
- Q: MVP size? → A: Moderate: the core workflow plus a few key options.
- Q: Platforms for v1? → A: macOS only.
- Q: Capture modes for v1? → A: Region select and active window.
- Q: How are captures triggered? → A: Global hotkeys.
- Q: Annotation tools for v1? → A: Arrows and shapes, text labels, numbered steps.
- Q: Where does the finished image go? → A: Clipboard and save to folder.
- Q: What makes it a developer tool? → A: Open source, small and fast. Agent-friendly features only if cheap.
- Q: Agent-friendly interface (headless CLI / MCP) in v1? → A: No. Move it all to the backlog.
- Q: Does every capture open the editor? → A: No. There are separate hotkeys for quick capture and for capture + edit.
- Q: File format and naming? → A: PNG, timestamped, in a configurable folder (default `~/Pictures/snapit/`).
- Q: Footprint targets? → A: Aggressive: idle memory < 20 MB, binary < 10 MB, hotkey to overlay < 100 ms.
- Q: Hard technical constraints? → A: Optimise for the lightest runtime and memory footprint.
- Q: Default hotkeys? → A: ⌘⇧X = region capture + annotate. ⌘⇧C = quick region capture to clipboard. The window-capture actions have no default binding; users can bind them in Preferences.
- Q: Should quick captures auto-save to the folder? → A: Clipboard only by default, with a setting to also save to the folder.
- Q: Implementation language? → A: Rust (changed from Go on 2026-11-21).
- Q: How is idle memory measured for SC-001? → A: Physical footprint (`phys_footprint`), not RSS (2026-11-21).

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Quick region capture to clipboard (Priority: P1)

A developer presses ⌘⇧C, drags a rectangle over part of the screen, and the image goes straight to the clipboard with no editor step. They paste it into Slack, a GitHub issue, or a chat with an AI assistant.

**Why this priority**: This is the most common screenshot action and the smallest slice that delivers value. Every other story builds on the capture pipeline it sets up.

**Independent Test**: With only this story built, press the quick-region hotkey, select an area, and paste into any app. The pasted image matches the selected area at full Retina resolution.

**Acceptance Scenarios**:

1. **Given** snapit is running in the menu bar, **When** the user presses the quick-region hotkey (default ⌘⇧C), **Then** a full-screen selection overlay appears within 100 ms with a crosshair cursor.
2. **Given** the selection overlay is showing, **When** the user drags a rectangle and releases, **Then** that area is captured at native pixel resolution and put on the clipboard as PNG. No file is written by default.
3. **Given** "Also save quick captures to folder" is on in Preferences, **When** a quick capture finishes, **Then** it is also saved to the save folder as a timestamped PNG.
4. **Given** the selection overlay is showing, **When** the user presses Esc, **Then** the overlay closes and nothing is captured, copied, or saved.
5. **Given** the user is dragging a selection, **When** the selection changes, **Then** the overlay shows the live pixel size of the selection (W × H).
6. **Given** a capture has finished, **When** it completes, **Then** the user gets brief, unobtrusive confirmation (for example, a flash of the menu bar icon or a short sound) and focus returns to the app they were using.

---

### User Story 2 — Capture + annotate (Priority: P2)

A developer presses the capture-and-edit hotkey, selects a region, and an editor opens. They add arrows, boxes, text, and numbered step markers, then copy or save the result.

**Why this priority**: Annotation is the main thing that sets snapit apart from the built-in macOS screenshot tool for bug reports and how-to guides. It depends on the capture pipeline from Story 1.

**Independent Test**: Press the capture-and-edit hotkey, select a region, add one item with each tool, press Done, and paste. The pasted image contains all the annotations, drawn at full resolution.

**Acceptance Scenarios**:

1. **Given** snapit is running, **When** the user presses the capture-and-edit hotkey (default ⌘⇧X) and finishes a selection, **Then** an editor window opens showing the captured image.
2. **Given** the editor is open, **When** the user picks the arrow, rectangle, ellipse, or line tool and drags on the image, **Then** that shape is drawn in the current color and stroke width.
3. **Given** the editor is open, **When** the user picks the text tool and clicks on the image, **Then** they can type a label in the current color and size and edit it later.
4. **Given** the editor is open, **When** the user picks the numbered-step tool and clicks repeatedly, **Then** markers appear numbered 1, 2, 3… in click order. Deleting a marker renumbers the ones after it.
5. **Given** annotations exist, **When** the user selects one, **Then** they can move it, delete it, or change its color.
6. **Given** annotations exist, **When** the user presses ⌘Z or ⇧⌘Z, **Then** the last action is undone or redone.
7. **Given** the editor is open, **When** the user presses Done or ⌘C, **Then** the flattened image is copied to the clipboard and the editor closes (Done) or stays open (⌘C).
8. **Given** the editor is open, **When** the user presses ⌘S, **Then** the flattened image is saved to the save folder as a timestamped PNG.
9. **Given** the editor has unsaved, uncopied changes, **When** the user closes it, **Then** they are asked whether to discard the changes.

---

### User Story 3 — Active window capture (Priority: P3)

A developer binds a window-capture hotkey in Preferences (quick or edit variant; no default binding), presses it, hovers over a window to highlight it, and clicks to capture just that window, cleanly cropped.

**Why this priority**: Window capture comes up often for app or terminal screenshots, but region capture can approximate it, so it ranks below Stories 1 and 2.

**Independent Test**: Bind and press the quick-window hotkey, click a Terminal window, and paste. The image contains only that window, even if other windows overlap it.

**Acceptance Scenarios**:

1. **Given** snapit is running, **When** the user presses a window-capture hotkey, **Then** the window under the cursor is highlighted as the cursor moves.
2. **Given** a window is highlighted, **When** the user clicks, **Then** only that window's contents are captured, without overlapping windows or desktop background.
3. **Given** the window-capture overlay is showing, **When** the user presses Esc, **Then** nothing is captured.
4. **Given** the quick-window hotkey was used, **Then** the result goes to the clipboard (and the save folder, if that setting is on), the same as Story 1. **Given** the window + edit hotkey was used, **Then** it opens in the editor, the same as Story 2.

---

### User Story 4 — First run, permissions, and preferences (Priority: P2)

A new user installs snapit, launches it, and is guided through granting Screen Recording permission. Later they change hotkeys and the save folder from a small preferences window opened from the menu bar.

**Why this priority**: Without permission, no capture works on macOS, so this is needed to ship. It ranks P2 because Story 1 can be developed and tested by granting permission by hand.

**Independent Test**: On a fresh macOS user account, launch snapit, follow the prompts, and complete a quick capture. Then change the save folder and confirm the next saved capture (⌘S in the editor) lands there.

**Acceptance Scenarios**:

1. **Given** Screen Recording permission has not been granted, **When** the user first triggers a capture or launches the app, **Then** snapit explains why the permission is needed and offers to open the right System Settings pane.
2. **Given** permission was denied or revoked, **When** a hotkey is pressed, **Then** snapit shows a clear message instead of failing silently or capturing a blank image.
3. **Given** snapit is running, **When** the user clicks the menu bar icon, **Then** a menu offers: Preferences…, Open Save Folder, About, and Quit.
4. **Given** the Preferences window is open, **When** the user records a new shortcut for any of the four capture actions, or clears one, **Then** the change takes effect immediately. If the new shortcut conflicts with an existing system or app shortcut, the user is warned.
5. **Given** the Preferences window is open, **When** the user picks a different save folder, **Then** later captures are saved there.
6. **Given** the Preferences window is open, **When** the user turns on "Also save quick captures to folder", **Then** later quick captures are copied to the clipboard and saved to the folder.
7. **Given** the Preferences window is open, **When** the user turns on "Launch at login", **Then** snapit starts automatically at the next login.

---

### Edge Cases

- **Multiple monitors / mixed DPI**: A region selection must work on whichever display the cursor is on. Images are captured at that display's native scale. Whether one selection can span two displays: see FR-005.
- **Zero-size or tiny selection**: A click without a drag (or a selection under 2×2 px) cancels rather than producing an empty image.
- **Save folder missing or not writable**: snapit recreates a missing default folder. If it can't write, the image still goes to the clipboard and the user is told the save failed.
- **Filename collision**: Two captures in the same second must not overwrite each other. Append a suffix (`-1`, `-2`).
- **Hotkey pressed while the overlay or editor is already open**: The press is ignored. It never stacks overlays.
- **Full-screen apps and Spaces**: The overlay appears on the active Space, including over full-screen apps.
- **Protected content** (DRM video, secure input fields): Whatever macOS returns is captured (often black). snapit does not crash.
- **Minimised or off-screen windows**: These are not offered as targets in window capture.
- **Very large captures** (for example, a 6K display region): The editor stays responsive and memory returns to near idle after the editor closes.

## Requirements *(mandatory)*

### Functional Requirements

**Capture**
- **FR-001**: The system MUST run as a menu bar–only app (no Dock icon) and stay resident in the background.
- **FR-002**: The system MUST support four capture actions, each with a configurable global hotkey: quick region, region + edit, quick window, and window + edit. Defaults: quick region = ⌘⇧C, region + edit = ⌘⇧X. The window actions are unbound by default.
- **FR-002a**: A global hotkey takes priority over the same shortcut in the frontmost app, for as long as it is bound. This matters for the defaults: ⌘⇧C is Inspect Element in Chrome DevTools, and ⌘⇧X is strikethrough in Slack. If a hotkey cannot be registered because another app or the system already owns it, snapit MUST tell the user and point them to Preferences.
- **FR-003**: The system MUST provide an interactive region selection overlay with crosshair, live dimensions, and Esc to cancel.
- **FR-004**: The system MUST provide window selection with hover highlight, capturing only the chosen window's contents.
- **FR-005**: Region selection MUST work on any connected display. A single region spanning several displays is out of scope for v1.
- **FR-006**: Captures MUST be stored at native pixel resolution (for example, 2× on Retina displays).
- **FR-007**: Window captures MUST NOT include the macOS window shadow in v1.

**Output**
- **FR-008**: Quick captures MUST be copied to the clipboard as PNG and MUST NOT be written to disk by default. An "Also save quick captures to folder" setting (default off) MUST also save them to the save folder.
- **FR-009**: Saved files MUST be PNG, named `snapit-YYYY-MM-DD-HHMMSS.png` (local time), with a numeric suffix on collision.
- **FR-010**: The save folder MUST default to `~/Pictures/snapit/` and be configurable.
- **FR-011**: Clipboard writes MUST paste correctly into common targets: Slack, GitHub (browser), VS Code, Notes, and Preview.

**Editor**
- **FR-012**: The editor MUST provide arrow, rectangle, ellipse, line, text, and numbered-step tools.
- **FR-013**: The editor MUST provide a small fixed palette (about 6 colors) and 2–3 stroke and text sizes.
- **FR-014**: Annotations MUST stay editable objects (select, move, delete, recolor) until export.
- **FR-015**: The editor MUST support undo and redo for all annotation actions.
- **FR-016**: Numbered steps MUST number themselves in sequence and renumber when one is deleted.
- **FR-017**: Export (Done, ⌘C, ⌘S) MUST flatten annotations onto the image at native resolution.
- **FR-018**: The editor MUST warn before discarding unexported changes.

**App & settings**
- **FR-019**: The system MUST detect missing Screen Recording permission and guide the user to grant it.
- **FR-020**: The system MUST provide a Preferences window for hotkeys, save folder, the "Also save quick captures to folder" setting, and launch at login.
- **FR-021**: Settings MUST persist across restarts.
- **FR-022**: The system MUST NOT make any network requests (no telemetry, no update pings) in v1.

### Key Entities

- **Capture**: One screenshot. It has the pixel data, source (region or window), source display, scale factor, timestamp, and, for window captures, the app or window title.
- **Annotation**: An editable object on top of a Capture. It has a type (arrow, rect, ellipse, line, text, step), geometry, color, size, text content (text type), and sequence number (step type).
- **Hotkey Binding**: A capture action (one of four) mapped to a key combination, or left unbound.
- **Settings**: Hotkey bindings, save folder, the save-quick-captures flag (default off), and the launch-at-login flag.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Idle memory (menu bar only, no overlay or editor open) is **under 20 MB** on Apple Silicon, measured as the process's physical footprint (`phys_footprint`, the "Memory" figure in Activity Monitor). RSS is not the metric: it counts system library pages shared with every app, so no AppKit app gets near 20 MB.
- **SC-002**: The release binary / app bundle is **under 10 MB**.
- **SC-003**: Time from hotkey press to visible selection overlay is **under 100 ms** (p95) on an M1-class Mac.
- **SC-004**: Idle CPU usage is effectively 0% (no polling loops). It averages under 0.1% over 10 minutes.
- **SC-005**: After closing the editor following a 5K capture, memory returns to within 5 MB of the idle baseline within 5 seconds.
- **SC-006**: A first-time user completes install, permission grant, and a first pasted capture in **under 2 minutes** with no outside docs.
- **SC-007**: Quick capture to clipboard (select, release, ready to paste) completes in **under 300 ms** for a full-display region.

## Assumptions

- **Platform**: macOS 14 (Sonoma) or later, on both Apple Silicon and Intel. The floor is set by ScreenCaptureKit's one-shot screenshot API (`SCScreenshotManager captureImageWithFilter:…`), which needs macOS 14. The older `CGWindowListCreateImage` is obsolete from macOS 15. See `research.md`.
- **Implementation direction** (see `plan.md`): Rust, calling AppKit and ScreenCaptureKit directly through the `objc2` bindings for the overlay, editor and capture. Cross-platform GUI toolkits (Fyne, Wails, Electron) are ruled out because they can't meet SC-001/SC-002.
- **Distribution**: Open source (license TBD), shipped as a signed and notarized `.app` through GitHub Releases and a Homebrew cask. Signing is needed for a stable Screen Recording permission grant.
- **Settings storage**: macOS user defaults or a small file under `~/Library/Application Support/snapit/`. Editing settings as a dotfile is not a v1 goal.
- The user has one active keyboard layout. Hotkeys are matched on key codes, not characters.

## Out of Scope (v1) / Backlog

- **Agent-friendly interface**: a headless `snapit capture … --json` CLI and an MCP server (`snapit mcp`) exposing capture and list-windows tools.
- Full-screen and delayed/timer capture modes.
- Blur/pixelate redaction, and auto-detecting secrets via OCR.
- Upload and copy link (S3, Imgur, custom host).
- Drag the image out of the editor into other apps.
- Markdown-snippet output (`![](path)`).
- Configurable filename patterns and JPEG/WebP output.
- Windows and Linux support.
- Scrolling capture, screen recording, and GIF.
- A region spanning more than one display.
- Window shadow option.
- Auto-update.
