# snapit

A tiny macOS menu bar screenshot tool for developers. Press a hotkey, drag a region, and paste. Or annotate it first with arrows, boxes, text and numbered steps.

- **Small**: a <1 MB app (0.5 MB binary) with 14 MB memory at idle, zero idle CPU and no network access. See [Performance](#performance).
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
make bench    # export and editor benchmarks (release build)
make measure  # bundle size, idle memory, CPU and sockets
```

On first launch, grant Screen Recording access in System Settings and relaunch. `make app` signs with a local self-signed certificate so the grant survives rebuilds (see the quickstart).

See [`specs/001-snapit-mvp/quickstart.md`](specs/001-snapit-mvp/quickstart.md) for setup, permissions and development notes. The spec, plan and tasks are in [`specs/001-snapit-mvp/`](specs/001-snapit-mvp/).

## Performance

Measured on an Apple M5 Pro running macOS 27.0.1, release build. Times are medians.

**At idle** (`make measure`, menu bar only):

| | snapit | Target |
|---|---|---|
| Memory (physical footprint, as in Activity Monitor) | 14 MB | < 20 MB |
| App bundle | 948 KB (binary 494 KB) | < 10 MB |
| CPU time over 30 s | 0 s | ≈ 0 |
| Network sockets | 0 | 0 |

**Quick capture**: what runs after you release the mouse, apart from the ScreenCaptureKit grab itself (`make bench`):

| | Retina (3024×1964) | 5K (5120×2880) |
|---|---|---|
| PNG encode, typical UI screenshot | 24 ms | 61 ms |
| PNG encode, worst case (every pixel random) | 369 ms | 921 ms |
| Clipboard write | < 0.1 ms | < 0.1 ms |

**Editor** (`make bench`, on a 5K capture):

| | 20 annotations | 100–200 annotations |
|---|---|---|
| Flatten onto the image for export | 4.7 ms | 9.0 ms (100) |
| Per mouse move while drawing | 0.2 µs | 1.3 µs (200) |
| Hit test | 0.1 µs | 0.8 µs (200) |

Encoding a typical screenshot takes well under the 300 ms capture-to-clipboard budget. Photo-like content with no flat areas is the PNG worst case and can go over it on large displays. Hotkey → overlay latency and the capture itself need real input and Screen Recording access, so they are checked by hand (see the quickstart).

## Layout

```
src/core/   pure Rust: settings, file naming, hotkey parsing, annotation model (unit-tested)
src/mac/    AppKit/ScreenCaptureKit: menu bar, Carbon hotkeys, overlay, capture, editor, renderer, prefs
src/app.rs  orchestration
```

## License

[MIT](LICENSE)
