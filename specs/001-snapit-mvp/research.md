# Research: snapit MVP

Findings were checked against the installed macOS 27.0 SDK (Command Line Tools, clang 21) on 2026-11-20.

## R1. Capture API → ScreenCaptureKit, macOS 14 floor

- `CGWindowListCreateImage` is marked `SCREEN_CAPTURE_OBSOLETE(10.5,14.0,15.0)`: deprecated in 14, obsolete in 15. It is not an option.
- `SCScreenshotManager +captureImageWithFilter:configuration:completionHandler:` is `API_AVAILABLE(macos(14.0))`. It returns a `CGImageRef` directly.
- `+captureImageInRect:` (15.2) and `+captureScreenshotWithFilter:` (26.0) are newer than we need.
- Supporting properties, all macOS 14: `SCContentFilter.pointPixelScale`, `.contentRect`, `SCStreamConfiguration.ignoreShadowsSingleWindow` and `.captureResolution`.
- **Decision**: the minimum is macOS 14.0 (spec updated).
  - Region capture uses `SCContentFilter initWithDisplay:excludingApplications:@[snapit]` with `sourceRect`. Excluding snapit means the dimmed overlay never appears in the image, even if it is still on screen.
  - Window capture uses `initWithDesktopIndependentWindow:` with `ignoreShadowsSingleWindow = YES` (FR-007).
- `SCShareableContent` (the list of displays and windows) is fetched asynchronously, which can take tens of ms. snapit starts the fetch when the overlay opens, so it is usually ready by the time the mouse is released (SC-007).

## R2. Global hotkeys → Carbon `RegisterEventHotKey`, prototypes declared by hand

- It is the only public API that both **consumes** the keystroke system-wide (FR-002a) and needs **no Accessibility permission**. `NSEvent addGlobalMonitorForEventsMatchingMask:` can only observe events, not consume them, and it needs Accessibility.
- The macOS 27 SDK's `HIToolbox/CarbonEvents.h` **no longer declares** `RegisterEventHotKey`, `InstallEventHandler` or `GetApplicationEventTarget`. `HIToolbox.tbd` still exports them.
- A spike that declared the prototypes by hand linked and returned `noErr` for ⌘⇧C. **Decision**: declare the prototypes in a Rust `extern "C"` block linked against Carbon.framework (`src/mac/hotkey.rs`).
  - **Risk**: Apple could remove the symbols. The fallback is an `NSEvent` monitor plus an Accessibility prompt.

## R3. Permission

- `CGPreflightScreenCaptureAccess()` checks without prompting. `CGRequestScreenCaptureAccess()` prompts once; after a denial, it never prompts again.
- Deep link to the settings pane: `x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture`.
- macOS usually needs the app relaunched after access is granted.

## R4. Code signing and TCC during development

- `security find-identity -v -p codesigning` → **0 identities** on this machine, so builds are signed ad hoc (`codesign -s -`).
- macOS ties the Screen Recording grant to the code signature, and an ad-hoc signature changes on every rebuild. **Each rebuild may need the grant again.**
- **Resolved**: `scripts/sign.sh` creates a self-signed `snapit-dev` certificate once, in a dedicated keychain, with OpenSSL and `security`. It signs every build with it, so the designated requirement (`identifier "dev.snapit.app" and certificate leaf = H"…"`) is stable and the grant survives rebuilds. codesign accepts the untrusted certificate, and nothing needs adding to the login keychain or trust settings.
- The app must be launched as a bundle (`open build/snapit.app`) so macOS records the grant against snapit and not the terminal.

## R5. Login item

- `SMAppService.mainAppService` with `registerAndReturnError:` and `unregisterAndReturnError:` (macOS 13). It needs a bundle and works with ad-hoc signing.

## R6. Keeping memory low

- Pixels stay as `CGImageRef` on the native side, from capture through editing to ImageIO PNG and the pasteboard. They are never copied into Rust-owned buffers, so memory drops quickly when the editor closes (SC-005).
- The overlay, editor and Preferences windows are created when needed and destroyed on close. Nothing is kept alive while idle.
- The UI is all native AppKit. GUI toolkits (Fyne, Gio, Wails) were rejected: GL/WebView runtimes alone are larger than the 20 MB idle target.

## R7. Rust bindings → `objc2` 0.6 family (spiked 2026-11-21)

- Crates: `objc2` 0.6.5, `objc2-foundation` / `objc2-app-kit` / `objc2-core-graphics` / `objc2-screen-capture-kit` 0.3.2, `block2` 0.6, `dispatch2` 0.3. Settings also use `serde` and `serde_json`.
- `SCScreenshotManager::captureImageWithFilter_configuration_completionHandler`, `SCStreamConfiguration::setIgnoreShadowsSingleWindow` and `SCContentFilter::pointPixelScale` are all bound.
- Subclasses (overlay panel, canvas view, delegates) use `define_class!`. AppKit calls require a `MainThreadMarker`, so the type system enforces the main-thread rule.
- **Spike result** (status item + Carbon hotkey, release build with `opt-level="z"`, fat LTO, strip):
  - Bundle 288 KB.
  - **phys_footprint 13 MB**, the figure Activity Monitor shows as "Memory".
  - RSS 49 MB, mostly shared AppKit pages from the system library cache. For comparison, Apple's own `TextInputMenuAgent` shows 36 MB RSS and an 18 MB footprint.
  - 0 CPU over 10 s and 0 sockets.
- **Implication for SC-001**: an RSS figure under 20 MB is out of reach for any AppKit app. snapit measures `phys_footprint` against the 20 MB target instead. The user approved this on 2026-11-21, and SC-001 now says so.
