# Architecture

## Pipeline

```
source (display | window | file | folder)
  |
  |  capture thread, own OS callback, latest frame wins
  v
frame ring buffer (depth 1)  --->  recorder (optional, disk)
  |
  |  inference thread
  v
resize / letterbox to 640 ---> detector (ONNX) ---> NMS ---> proposals
  |
  v
crop + batch encode (ONNX) ---> match against project pool ---> scored boxes
  |
  |  tracker: IoU association, ids, dwell counter
  v
emit at 30 Hz, normalized coords ---> overlay webview (canvas)
```

Three threads, one direction, no shared mutable state beyond the ring buffer. The overlay never asks for data; it receives it.

**Latest frame wins.** The capture callback writes into a single slot and bumps a generation counter. If inference is still busy when the next frame arrives, the older frame is dropped. A queue is the wrong structure here: latency matters more than coverage, and a queue turns a slow GPU into a growing delay between what you see and what is boxed.

**Two clocks.** Capture runs as fast as the source allows. Emission is capped at 30 Hz because a webview repainting 60 times a second for twenty rectangles costs more than it shows.

## Why not OpenCV

The `opencv` crate is maintained, current, and crosses the FFI boundary into a native library you must install and then ship. On macOS that means a framework or a Homebrew dependency, on Windows a Chocolatey install, and at release time a signed, notarized set of dylibs inside the bundle. Its own README calls the API "usable, but unstable and not very battle-tested".

Nothing in this app needs OpenCV. The operations are: decode an image, resize, crop, convert BGRA to RGB, letterbox, non-maximum suppression, blur, and a normalized cross-correlation for HUD templates. All of that exists in pure Rust crates, compiles into the binary, and ships as one file.

If a later feature genuinely needs a full CV library (calibration, optical flow, homography), the decision should be revisited with that feature on the table, not pre-emptively.

## Crates

| Need | Crate | Note |
| --- | --- | --- |
| App shell | `tauri` v2 | Windows, overlay window, updater support |
| Capture | `scap` | DXGI or Windows Graphics Capture, ScreenCaptureKit, X11. Validate in the spike; `xcap` is the fallback |
| Inference | `ort` v2 | ONNX Runtime 1.31, prebuilt binaries, CPU plus CoreML or DirectML execution providers |
| Image ops | `image`, `fast_image_resize`, `imageproc` | Decode, resize, blur, NCC |
| Numerics | `ndarray` | Tensors in and out of `ort` |
| Storage | `rusqlite` with `bundled` | No system SQLite anywhere |
| Serialization | `serde`, `serde_json` | IPC and project files |
| Time | `time` or `chrono` | Timestamps, ISO 8601 on disk |

Front end: React 19, TypeScript, Vite, Tailwind, shadcn/ui components, `lucide` icons through the shadcn set, TanStack Query for command results, Zustand for live session state.

## Capture

| Platform | API | Permission | Caveat |
| --- | --- | --- | --- |
| Windows 10 1903+ | Windows Graphics Capture, fallback DXGI Desktop Duplication | None | Exclusive fullscreen may return black or fail; borderless windowed is the supported path |
| Windows, older | DXGI Desktop Duplication | None | Fails on some hybrid GPU laptops, needs a restart path |
| macOS 12.3+ | ScreenCaptureKit | Screen Recording, TCC, per binary | Prompt needs `NSScreenCaptureUsageDescription`; app is killed if it triggers the prompt without it |
| Linux | PipeWire or X11 | Portal consent | Out of scope for v1 |

Capture of protected content (DRM video, some launchers) returns black frames on every platform. Treat black frames as a detectable state and say so in the UI instead of drawing nothing.

### Grabbing without the app in frame

The grab is one call, `grab::grab(&app)`, and the platform decides how the app stays out of the picture. A `ScreenSource` reports whether its capture already excludes this app's windows:

- `Direct`: the capture leaves the windows out, so the frame comes back untouched.
- `HideFirst`: the composited display includes them, so every visible iRecon window hides, a reference frame is taken, the screen is read up to four times, and the grab is accepted only when more than 0.4 percent of pixels changed. Windows are restored on every path. A grab that never sees that change saves nothing and returns the reason.

Today every platform is `HideFirst`, because `xcap` reads the composited display. The two backends that flip it without touching the call site:

| Platform | Backend | Exclusion |
| --- | --- | --- |
| Windows 10 2004+ | Windows Graphics Capture, or DXGI as a fallback | `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` on each iRecon window |
| macOS 12.3+ | ScreenCaptureKit | `SCContentFilter(display:excludingApplications:exceptingWindows:)` |

The pixel comparison is not a stand-in for a broken backend. It stays because a window's visibility flag says nothing about what the compositor has already drawn, and because a native backend can still fail to exclude a window it does not own. The proof is the same code on both paths, and the tests for it run without a display.

## Overlay

A second Tauri window, not a child of the main one: `transparent: true`, `decorations: false`, `always_on_top: true`, `skip_taskbar: true`, `focus: false`, sized to the captured display.

| Concern | Windows | macOS |
| --- | --- | --- |
| Click through | `WS_EX_TRANSPARENT` plus `WS_EX_LAYERED`, or `set_ignore_cursor_events(true)` | `set_ignore_cursor_events(true)` sets `ignoresMouseEvents` |
| Above fullscreen apps | Works over borderless windowed; exclusive fullscreen is out of reach | Needs an `NSPanel` (the `tauri-nspanel` plugin) and a non-activating style mask; a normal window stays on the desktop's space |
| Coordinates | Physical pixels per monitor, DPI aware | Points, with a separate scale factor per display |
| Capture of the overlay itself | Excluded if the app captures its own window, otherwise it appears in the capture | Same; the overlay must be excluded from capture or it feeds back |

**Coordinates never leave Rust as pixels.** Detections are normalized to `0..1` against the captured frame, and the overlay multiplies by its own client size. This kills the whole class of DPI and multi-monitor scaling bugs, and it means a detection recorded at 1080p still lands correctly on a 1440p overlay.

**Feedback loop.** If the overlay is visible and the capture source is the whole display, the app captures its own boxes. Two options: set the overlay window's capture exclusion (`SetWindowDisplayAffinity` with `WDA_EXCLUDEFROMCAPTURE` on Windows, filters on macOS), or subtract last frame's boxes from the proposal pass. Prefer exclusion; the subtraction version produces self-reinforcing artifacts.

## Models

| Role | Model | Size | Latency at 640, CPU | License |
| --- | --- | --- | --- | --- |
| Proposals | RF-DETR Nano at 384, or Small at 512, ONNX | 108 or 114 MB | 15 to 40 ms on CPU, 5 to 15 with CoreML | Apache-2.0, ships in a closed app |
| Embedding | SigLIP or CLIP image encoder, small variant, ONNX | 20 to 90 MB | 3 to 8 ms per batch of 8 crops | SigLIP Apache-2.0, CLIP MIT |
| HUD templates | User crops | negligible | under 1 ms | user's own |

**Licensing is a shipping decision, not a detail.** Ultralytics models are AGPL-3.0: a distributed closed-source app that bundles them needs their commercial licence. The choice is recorded in [models](05-models.md): RF-DETR, Apache 2.0, no NMS, offered as Nano at 384 and Small at 512.

Both sizes are pinned in a catalog beside the app: a row per file with its own URL, byte count, SHA-256 and input size. The Capture lab downloads, verifies and loads one at a time, and the chosen id goes into `models/active.txt` so the next start reloads the same graph. Nothing is fetched from a registry at runtime. Deciding this after the detector is wired in would mean re-testing detection quality against a different model.

Embeddings are stored as little-endian `f32` blobs with the model name beside them. Changing encoder invalidates the pool, so the encoder is versioned per project and the app re-embeds examples in the background when it changes.

## Matching and export

Live capture reloads the project pool every two seconds and crops every proposal out of the full frame. A proposal seeds a match only when it clears both the detector's primary floor and its class's threshold; a tiny seed may then grow to a larger enclosing proposal that scores within 0.05, and overlapping seeds are deduplicated before reaching the overlay. Proposals below the primary floor can support a seed but never become matches on their own, which keeps rock and wall texture out. A class with no examples cannot match, so an empty project draws nothing.

Surviving boxes go through an IoU tracker: same label, at least 0.3 overlap, greedy best match, coordinates halved toward each new detection so a box stops jittering, and a track coasts through up to two missed frames once it has been seen three times. Each emitted box carries a `track` id and a `seen` count, which is the identity a later automation rule will refer to.

An example is reduced to a 448-dimension fingerprint (`irecon-fingerprint-v3`): red and green ratios, luma, edge strength and a saturation-weighted hue direction on an 8 by 8 grid, plus a 64-bin global colour histogram, with the average cell subtracted before normalising. The subtraction is what makes it work. Without it every pair of crops scored around 0.99, so no threshold could tell a match from a near miss. It is stored as a little-endian `f32` blob with the model name beside it, so replacing the fingerprint with an ONNX encoder changes the name and forces a lazy re-embed instead of comparing across two vector spaces. Scoring takes the best positive per class, subtracts a quarter of the best negative, and each class's own threshold decides whether its box reaches the overlay.

Each class's threshold is adjustable on the Label page in 5 percent steps, and the capture loop's pool reload applies it within two seconds.

The default threshold is 0.85. `cargo run --example match_bench -- --demo` measured 72 synthetic crops in six classes plus 12 unknown blobs: top-1 was 1.000, correct crops averaged 0.997 against 0.000 for wrong ones, and 0.85 kept every correct crop while holding false accepts under 5 percent, where the old 0.55 default accepted all 12 blobs. Point the same tool at a folder of real crops, one subfolder per class and `__none__` for unknown objects, to re-tune on gameplay rather than synthetic shapes.

Dataset export writes a YOLO detection layout: one image and one label file per source frame, `classes.txt`, `data.yaml`, the crops under `crops/<class>/`, and a `manifest.json` with counts, class indices, per-frame box counts and every skipped row. Examples with no source frame or an unreadable box are counted as skipped, never dropped silently.

The agent surface (`agent_snapshot`, `agent_act`) exists only in development builds. It runs the same functions the UI calls, returns JSON instead of pixels, and appends one line per action to `agent-trace.jsonl` in the project so a run can be audited afterwards.

## Data model

Per project, one SQLite file.

```sql
create table project (
  id integer primary key,
  name text not null,
  slug text not null unique,
  created_at text not null,
  notes text
);

create table source (
  id integer primary key,
  project_id integer references project(id) on delete cascade,
  kind text not null check (kind in ('display','window','file','folder')),
  label text not null,
  config text not null,
  crop_x integer, crop_y integer, crop_w integer, crop_h integer
);

create table class (
  id integer primary key,
  project_id integer references project(id) on delete cascade,
  name text not null,
  color text not null,
  threshold real not null default 0.75,
  unique (project_id, name)
);

create table example (
  id integer primary key,
  class_id integer references class(id) on delete cascade,
  path text not null,
  role text not null check (role in ('positive','negative')),
  origin text not null check (origin in ('import','draw','harvest')),
  bbox text,
  embedding blob,
  embedding_model text,
  created_at text not null
);

create table candidate (
  id integer primary key,
  project_id integer references project(id) on delete cascade,
  class_id integer references class(id) on delete cascade,
  path text not null,
  score real not null,
  state text not null default 'pending'
    check (state in ('pending','accepted','rejected','skipped')),
  created_at text not null
);

create table session (
  id integer primary key,
  project_id integer references project(id) on delete cascade,
  started_at text not null,
  ended_at text,
  source_id integer,
  stats text
);

create index example_class on example(class_id);
create index candidate_pending on candidate(project_id, state);
```

Matching is brute force: normalize each embedding, dot product against the pool, take the best positive and the best negative, score is the margin. A few thousand vectors at 512 dimensions is a fraction of a millisecond, so a vector index is unnecessary until the pool is far larger than any hand-labelled project will get. Revisit with `sqlite-vec` if a project exceeds about 50k examples.

## On disk

```
~/Library/Application Support/iRecon/        (macOS)
%APPDATA%\iRecon\                            (Windows)
  projects/
    traffic-watch/
      project.json
      project.sqlite
      examples/<class>/<uuid>.png
      candidates/2026-11-07/<uuid>.png
      sessions/2026-11-07T20-14-02/
        meta.json
        frames/000001.jpg
        detections.jsonl
      models/custom-detector.onnx
  models/
    detector-yolo11n.onnx
    encoder-siglip-base.onnx
  logs/
```

Frames are JPEG at quality 85, written only when a session is recording. A 60 second session at 10 fps is about 60 MB, which is why recording defaults to off and prunes to the last N sessions.

## IPC

Commands (front end calls Rust):

| Command | Returns |
| --- | --- |
| `project_list` / `project_create` / `project_open` | project rows |
| `source_list` / `source_test` | one frame plus its size, for the preview |
| `class_create` / `class_update` | class row |
| `example_import(paths, class_id)` | inserted count, per file errors |
| `example_delete(id)` | ok |
| `candidate_list(state)` / `candidate_resolve(id, decision)` | queue |
| `session_start(project, source, classes)` / `session_stop` | session row |
| `engine_status` | device, model names, current fps, last inference ms |
| `match_region(project, frame, box)` | ranked classes with score, margin and the example that matched |
| `dataset_export(project)` | path, frame and box counts, skipped rows |
| `agent_snapshot` / `agent_act` | development builds only; JSON state, JSON actions, one trace line each |

Events (Rust pushes front end):

```
iRecon://detections  { t: u64, frame: u64, boxes: [ { track, class, x, y, w, h, score } ] }
iRecon://status      { fps, capture_ms, infer_ms, dropped, device }
iRecon://candidate   { class, path, score, nearest: [example_id] }
```

Boxes carry normalized coordinates and a `track` id, never pixels and never absolute screen positions. The overlay is a thin renderer; all geometry decisions live in Rust so the live view and any later automation agree about where things are.

## Performance budget

Target: 30 fps end to end at 1080p on a laptop, 60 fps capture with dropped frames when inference cannot keep up.

| Stage | Budget |
| --- | --- |
| Capture callback to buffer | 3 to 8 ms |
| Letterbox to 640 | 1 to 2 ms |
| Detector | 10 to 25 ms |
| Crop and encode 12 boxes | 4 to 10 ms |
| Match | under 1 ms |
| Tracker and emit | 1 to 3 ms |
| Overlay paint | 2 to 5 ms |

Over 33 ms means reduce detector input to 512, encode crops in one batch, or drop proposals below a minimum box size before encoding. The last one is free accuracy: a 12 pixel box is not worth 8 ms of encoder time.

## Security and privacy

Local only. No telemetry, no network calls except an optional model download on first run, which must be a visible, cancellable action with a hash recorded.

No process injection, no memory reads, no hooks into the target application. The app reads pixels through the OS capture API and draws its own window. That is the boundary, and it is the boundary that keeps it usable in games with anti-cheat: a screen overlay that never touches the game process is a very different thing from an injected DLL, though a game's terms of service can still forbid it. Say so in the app once, at first run, in plain language.

Screenshots and recordings are user data in the user's Application Support folder. Nothing is encrypted; the app should not imply otherwise. A project delete removes the folder and the database, and should say exactly what it removes.
