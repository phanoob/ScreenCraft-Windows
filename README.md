# ScreenCraft

**English** · [Français](README.fr.md)

A screen recorder and video editor for macOS and Windows, written in Rust. Record your display,
a window or an area (with an optional camera bubble, microphone and system audio),
then turn the raw capture into a polished video: smooth automatic zooms that follow
your cursor and typing, restyled cursor, backgrounds, rounded corners, shadows,
cuts, speed changes, background music and MP4 / GIF export.

There are no AI features: just recording, camera, editing and zoom.

> **Windows port of the original macOS app.** ScreenCraft was created by
> **Marshall Nickolauson** as a macOS app:
> [MarshallNickolauson/ScreenCraft](https://github.com/MarshallNickolauson/ScreenCraft)
> (MIT license, see [LICENSE](LICENSE)). This repository keeps its full history and
> its macOS code unchanged, and adds a Windows back end (`src-tauri/src/win`) plus the
> small UI changes Windows needs. Credit for the app, its design and the macOS version
> goes to the original author; issues with the Windows port belong here, not upstream.

> **Not affiliated.** ScreenCraft is an independent, open project that is
> *inspired by* the idea behind [Screen Studio](https://screen.studio). It is not
> affiliated with, endorsed by, or connected to Screen Studio or its makers in any
> way. "Screen Studio" is a trademark of its respective owner. ScreenCraft contains no
> code, artwork or other assets from that product.

## Screenshots

The editor, with a generated demo recording (a synthetic "app" with a typing
box, not real footage). Zoom regions are the blue blocks on the timeline.

![The editor](docs/screenshots/editor.jpg)

Paused inside a zoom: the camera is framing the cards and the preview re-fetches a
full-resolution frame so zoomed stills stay sharp.

![Zoomed in the editor](docs/screenshots/editor-zoom.jpg)

Cursor and camera-motion settings:

![Cursor and camera motion](docs/screenshots/editor-cursor.jpg)

Export, with optional compress-after-export and a size estimate:

![Export dialog](docs/screenshots/export.jpg)

The recorder bar: capture mode, camera / microphone / system audio with level
meters, resolution and frame rate, and the project list.

![Recorder bar](docs/screenshots/recorder.jpg)

## What it does

**Recording**
- Capture a full display, a single window, or a dragged area (the area outside it is
  dimmed so you can see what is being recorded).
- Camera bubble, microphone and system audio, each with a device picker and a live
  level meter. Choose resolution (Native / 2560 / 1920 / 1600 / 1280) and 30 or 60 fps.
- The system cursor is not baked into the video, so it can be redrawn, smoothed and
  restyled in the editor. Clicks, cursor motion and keystrokes are logged alongside.
- Draggable recording HUD, countdown, global shortcut **⌘⇧2**, menu-bar controls.

**Editing**
- **Zoom:** automatic zoom regions generated from your clicks and typing, with
  adjustable ease in / ease out, plus manual zooms you can place anywhere. The camera
  keeps the cursor near the centre and, while you type, follows the text caret.
- **Timeline:** video, zoom and music tracks. Split, trim, reorder, duplicate and
  delete clips; change speed per clip; drag zoom regions and music clips freely.
- **Look:** wallpapers, gradients, solid colours or your own image (with blur),
  padding, corner radius, shadow, aspect ratio, crop.
- **Cursor:** several styles, click effects, hide-when-idle, hides while you type.
- **Camera bubble:** size, shape, position, mirror, border, and it shrinks smoothly
  while the view is zoomed in.
- **Audio:** separate mic / device / music / master levels. The microphone is
  levelled and centred automatically.
- **Music:** add an audio file, trim and split it, see its waveform on the track.
- **Project files:** edits stay in memory until you save (**⌘S**); a `*` in the title
  shows unsaved changes and closing or quitting asks before discarding them.

**Export & compression**
- MP4 (H.264 / HEVC) or GIF at 720p – 4K, 30 / 60 fps, with a size estimate.
- Optional *compress after export* (HandBrake-style presets via AVFoundation), with
  the choice to keep the original, the compressed copy, or both.
- A standalone **Compress Video…** tool for any video file.

**Languages**
- The interface is available in English, French and German. It follows the system
  language; change it under **Language** in the Projects menu (recorder bar or editor).

## Install

ScreenCraft is built from source; there is no signed release download yet.

### macOS

**Requirements**
- macOS 15 (Sequoia) or newer (developed and tested on Apple silicon)
- [Rust](https://rustup.rs) (stable) and the Xcode command line tools
  (`xcode-select --install`)

```bash
git clone https://github.com/MarshallNickolauson/ScreenCraft.git
cd ScreenCraft

./scripts/bundle.sh --install     # builds a release app and copies it to ~/Applications
open ~/Applications/ScreenCraft.app
```

`./scripts/bundle.sh` without `--install` leaves the app at `dist/ScreenCraft.app`.
The first release build takes a few minutes.

**Permissions.** On first launch a setup window checks the macOS permissions the app
needs and takes you to the right System Settings pane:

| Permission | Why |
| --- | --- |
| Screen Recording | capturing the display, windows and areas (required) |
| Accessibility | finding the text caret so the camera can follow your typing (required) |
| Input Monitoring | noticing when you start typing (required) |
| Camera / Microphone | only if you use the camera bubble or narration |

macOS applies Screen Recording and Input Monitoring after the app restarts; the
window offers a restart button. The app is signed ad hoc with a stable identity, so
permissions persist across rebuilds.

### Windows

**Requirements**
- Windows 10 2004 or newer (Windows 11 recommended: it can hide the yellow capture border)
- [Rust](https://rustup.rs) (stable) and the Visual Studio Build Tools with the
  "Desktop development with C++" workload
- The WebView2 runtime (preinstalled on Windows 11)

```powershell
git clone https://github.com/phanoob/ScreenCraft-Windows.git
cd ScreenCraft-Windows

.\scripts\build-windows.ps1            # builds dist\ScreenCraft.exe
.\scripts\build-windows.ps1 -Install   # also installs it with a Start menu shortcut
```

No permissions need granting on Windows; camera and microphone follow the switches
in Settings › Privacy & security. The recording shortcut is **Ctrl+Shift+2**, the
other shortcuts use Ctrl where macOS uses ⌘, and the app lives in the notification
area (system tray) instead of the menu bar. Settings are stored in
`%APPDATA%\ScreenCraft`; recordings and exports go to `Videos\ScreenCraft`.

## Using it

1. Pick **Display / Window / Area**, choose camera and audio sources, press **Record**
   (or **⌘⇧2**). Press **Stop** in the HUD or use the shortcut again.
2. The recording opens in the editor. The first open builds a smooth preview copy of
   the footage, which takes a little while for long recordings and is then reused.
3. Adjust zooms, look and audio; add music if you like; **⌘S** to save.
4. **Export** (top right). Files go to `~/Movies` by default.

Projects live in `~/ScreenCraft Projects/<name>.screencraft` and can be renamed by
clicking the title in the editor.

### Shortcuts

`Space` play / pause · `S` split at playhead · `Z` add zoom · `D` duplicate ·
`⌫` delete selection · `←` `→` step · `⌘Z` / `⇧⌘Z` undo / redo · `⌘S` save ·
`⌘⇧2` start / stop recording

## How it is built

| Piece | Where | What it does |
| --- | --- | --- |
| `screencraft-core` | `crates/core` | Pure Rust: project model, spring physics, timeline mapping, auto-zoom generation and the per-frame camera / cursor render track. Unit tested. |
| App shell | `src-tauri` | Tauri 2 app: ScreenCaptureKit capture, input and caret tracking, project IO, native preview proxy and frame server, audio levelling, AVFoundation compression, windows and menus. |
| UI | `ui` | Plain ES modules, no bundler: recorder bar, editor, canvas compositor and timeline. Exports with WebCodecs. Translations in `ui/js/locales` (see `ui/js/i18n.js`). |
| Windows back end | `src-tauri/src/win` | Swapped in one-for-one for the macOS modules: Windows Graphics Capture (`windows-capture`) + Media Foundation H.264/AAC for recording, WASAPI loopback for system audio, Win32 hooks and UI Automation for input and caret tracking, Media Foundation decoding for proxies, export frames and compression. |

Some details worth knowing:

- **Preview proxy.** The editor never plays a `<video>` element. Rust decodes the
  recording once into a small JPEG strip next to the project and serves frames on
  demand over a custom `sc://` scheme; audio is decoded and scheduled with WebAudio.
  This avoids a WebKit / AVFoundation player deadlock and makes scrubbing instant.
  Paused frames are re-fetched at full resolution.
- **Export** reads full-resolution source frames natively, paints them with the same
  compositor code as the preview, and encodes with WebCodecs plus a bundled MP4
  muxer (`ui/vendor`, MIT). No ffmpeg is required.
- **Wallpapers** are generated procedurally; no third-party artwork is bundled.

A `.screencraft` package contains `screen.mp4`, optional `camera.mp4` and `mic.m4a`,
`input.json` (cursor, clicks, keys, caret), `project.json`, and generated
`proxy-*` / `mic-leveled-*` cache files that are safe to delete.

## Development

```bash
cargo test -p screencraft-core     # unit tests for the core crate
cargo check -p screencraft         # type-check the app
./scripts/bundle.sh --install      # rebuild and reinstall
```

## License

MIT, see [LICENSE](LICENSE).

## Known limitations

- macOS and Windows only (no Linux back end).
- On Windows, the H.264 encoder caps recordings at 4096 pixels on the long side, and
  system audio cannot leave out ScreenCraft's own sounds (macOS can).
- Preview proxies are large (roughly 2–3 MB per second of recording); delete the
  `proxy-*` files in a project folder to reclaim space.
- Export is not real-time; long 60 fps projects take a while.
- Typing focus depends on the focused app exposing its text caret through
  Accessibility (UI Automation on Windows); otherwise the position is estimated
  from keystrokes.
- No code signing or notarisation, so you build it yourself.
