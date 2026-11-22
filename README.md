# snapit

A tiny macOS menu bar screenshot tool for developers. Press a hotkey, drag a region, and paste. Or annotate it first with arrows, boxes, text and numbered steps.

- **Small**: a ~0.5 MB app with ~14 MB memory at idle, zero idle CPU and no network access.
- **Native**: Rust calling AppKit and ScreenCaptureKit through [`objc2`](https://github.com/madsmtm/objc2). No Electron, no WebView.
- **macOS 14+**

| | |
|---|---|
| ⌘⇧C | capture a region to the clipboard |
| ⌘⇧X | capture a region and annotate |

Window capture is available in Preferences.

## Build

Requires macOS 14+, Rust 1.88+ and the Xcode Command Line Tools.

```sh
make run      # build, bundle, sign and launch build/snapit.app
make test
```

On first launch, grant Screen Recording access in System Settings and relaunch. `make app` signs with a local self-signed certificate so the grant survives rebuilds (see the quickstart).

See [`specs/001-snapit-mvp/quickstart.md`](specs/001-snapit-mvp/quickstart.md) for setup, permissions and development notes. The spec, plan and tasks are in [`specs/001-snapit-mvp/`](specs/001-snapit-mvp/).

## Layout

```
src/core/   pure Rust: settings, file naming, hotkey parsing, annotation model (unit-tested)
src/mac/    AppKit/ScreenCaptureKit: menu bar, Carbon hotkeys, overlay, capture, editor, renderer, prefs
src/app.rs  orchestration
```

## License

[MIT](LICENSE)
