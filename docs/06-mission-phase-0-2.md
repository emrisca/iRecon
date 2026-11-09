# Mission: finish Phase 0, lock the base, then Phase 1 and 2

This is the working plan for the only phase that matters right now. Phase 0 to 2
is the base of the product: if the overlay cannot sit over a running game and the
live loop cannot keep up with the screen, nothing built on top of it is worth
building. Phases 3 to 6 stay closed until the gates below pass.

Each mission has one question, one experiment, one gate, and a fallback. A
mission is done when its gate has a recorded number, not when the code compiles.

## What is already in the tree

Phase 0: Tauri v2 shell, primary display capture, transparent click-through
topmost overlay, boxes, fps and cost readouts.

Phase 1: project CRUD, source picker with preview and crop, library import,
classes with thresholds, example editor, review queue.

Phase 2: capture thread, RF-DETR proposals, matcher, tracker, overlay events,
status bar, session recording and replay, candidate harvesting.

## The measurement that framed the plan

`capture_benchmark` on the 2560 by 1440 primary display, before this mission:

| Case | Pixels | Time |
| --- | --- | --- |
| Full display | 3.69 M | 205.6 ms |
| Half width, half height | 0.92 M | 55.4 ms |
| 1280 by 720 centre | 0.92 M | 60.0 ms |

Fit: `cost = 6 ms + 200 ms * area`, area as a fraction of the display, about
54 ns per pixel. That is not the cost of moving pixels, it is a still-image path
paying setup and teardown every frame. Cropping cannot save it: 16.7 ms would
need 5 percent of the display, roughly 560 by 470.

Second number: inference 75.6 ms on the CPU execution provider, which caps a
single-threaded loop near 13 fps even with free capture. Capture rate and
detection rate therefore became two clocks.

## Gates

| Gate | Target | Method |
| --- | --- | --- |
| G1 capture rate | 55 fps or better, 60 s sustained, full display | stats from the capture thread |
| G2 capture cost | 12 ms p50, 20 ms p95 per frame | per frame histogram |
| G3 latency | frame to overlay under 130 ms p95 | capture timestamp to publish |
| G4 paint rate | 55 fps or better with 20 boxes, capture idle | sine-wave calibration mode |
| G5 soak | 10 minutes, RSS growth under 50 MB, no click eaten, under 1 percent dropped frames | counters |
| G6 hotkey | toggles overlay under 100 ms with the game focused | manual, both directions |
| G7 recall | 80 percent of visible NPC cars boxed over 100 frames | recorded session, manual count |
| G8 Phase 2 exit | 20 fps or better with a 20 example class, empty class draws nothing | live plus replay |

## Architecture: three clocks

```
capture clock   60 Hz   duplication session -> RGBA -> latest slot
                        never blocks, no per frame allocation, drops old frames

inference clock ~12 Hz  newest frame -> preprocess, run, match, track,
                        publish boxes with the capture timestamp

paint clock     60 Hz   overlay draws newest boxes, extrapolates them by age,
                        clears when stale
```

Rules: one frame in the slot, producer overwrites, consumer takes, so latency
cannot accumulate and a slow model never pushes capture behind. Capture never
encodes, resizes, or touches SQLite. Preview encoding sits on the inference
clock. A crop is carried with the frame and applied by consumers, so the overlay
and the preview agree.

## Novel approaches, ranked

1. Streaming duplication instead of still capture. `xcap` 0.9.8 exposes
   `Monitor::video_recorder()`, which on Windows is DXGI Desktop Duplication.
   Setup is paid once and frames arrive on a channel.
2. Windows Graphics Capture as the second source, through `xcap`'s `wgc`
   feature or the `windows-capture` crate. Survives modes duplication does not
   and can capture one window.
3. Split the clocks, so a 75 ms model coexists with 60 fps capture.
4. Temporal amortisation: detect at the rate the model allows, let paint
   extrapolate between detections.
5. GPU execution provider for ONNX Runtime, probed and reported, never required.
6. Preprocess cost measured on its own instead of hidden inside a frame budget.
7. Allocation free frame path: shared frame buffers, no proposal clones,
   row copies instead of per-pixel crops.
8. Paint budget measured alone, then trimmed: no per frame canvas resize,
   desynchronised context, extrapolation instead of repaint per detection.
9. Measure release only.
10. Interference: Game Bar or recording software holding a duplication session,
    GPU preference per app, borderless windowed rather than exclusive fullscreen.

## Missions

- M0 instrumentation: per frame histogram with p50/p95/max, drop count, and
  separate timings for preprocess, inference, match, and latency.
- M1 capture sprint: E1 duplication recorder, E2 Graphics Capture behind a
  feature, E3 still capture with the `wgc` feature, E4 the winning path on the
  frame the app actually uses.
- M2 three clocks: `FrameSlot`, inference thread, capture thread, preview and
  recording moved off the capture clock.
- M3 Phase 0 tools: global hotkey, sine-wave calibration with paint numbers,
  click-through after every show, soak counters.
- M4 detector audit: score the shipped model over recorded frames and report
  proposals and coverage so the 80 percent question can be answered.
- M5 Phase 2 replay: file and folder sources through the same pipeline at a
  fixed 20 Hz, plus a source picker in the capture lab.
- M6 verification: tests, build, docs, measured results.

## Measurement protocol

Release build, game borderless, same scene. Warm up, then sample 60 s. Report
p50, p95 and max, never a bare average. Capture fps counts frames actually
delivered. Latency is stamped at capture and again when boxes are published.

## Measured results

Machine: 2560 by 1440 primary display at 143 Hz, GeForce RTX 4060 Ti, Windows.
Release build, `cargo run --release --bin capture_probe -- 12`, real motion on
the display.

### M1 capture sprint

| Backend | Frames in 12 s | Rate | Gap p50 | Gap p95 | Note |
| --- | --- | --- | --- | --- | --- |
| Still capture, `wgc` feature on | 460 | 38.3 fps | 25.6 ms cost p50 | 34.5 ms | 205.6 ms per frame before the feature |
| Desktop Duplication, `xcap` recorder | 637 | 53.0 fps | 20.0 ms | 26.4 ms | 14.7 MB RGBA frame |
| Graphics Capture, `windows-capture` | 1163 | 106.0 fps | 8.0 ms | 12.5 ms | 1 ms minimum update interval |

E3 answered a question the plan only guessed at. Turning on `xcap`'s `wgc`
feature took the still path from 205.6 ms to 25.6 ms, an eightfold change with
no other edit. So the original cost was the GDI readback path, not the pixels,
and the earlier area-fit was describing an implementation rather than a limit.

E1 answered the plan's main question. Duplication delivers 53 fps with a 20 ms
frame gap on a 143 Hz display: real streaming, but short of G1 and apparently
capped by the duplication session rather than by our code.

E2 found the winner. Graphics Capture delivers 106 to 116 fps with an 8 ms gap
because it follows the compositor instead of a fixed 60 Hz handoff. The live
loop now uses it for the primary display, capped at a 15 ms minimum update
interval so a 143 Hz display does not hand the app 1.7 GB/s of frames;
duplication is the second path for a non-primary display, and still capture
remains the last resort with a note that says so. The cap started at 16 ms; the
soak run below is what moved it to 15.

E4 turned out not to need its own run. The winning path was already paying the
full RGBA copy on every frame, 1.7 GB/s of rows at 116 fps, and still held an
8 ms gap, so handoff is not the limiter at the rates this app cares about. The
same copy at the 60 fps cap is about 880 MB/s, which is the number the app
actually spends.

E2 and the copy figure come from the probe, which drives the identical settings
and the identical row copy the app's handler uses. The in-app armed run with a
game, which is what G1 and G2 are written against, still needs a play session.

### Gate status

| Gate | Status |
| --- | --- |
| G1 capture rate | **Met** by the ten minute harness run: 59.8 fps over the run and over its last 60 s, full display, Graphics Capture. Confirming it with a game on the same path has not been run yet; it awaits a play session. |
| G2 capture cost | **Met**: p50 4.62 ms, p95 4.91 ms, max 9.61 ms across 600 s. Same caveat as G1: compositor motion, not a game. |
| G3 latency | Instrumented end to end. The pipeline leg before paint measured p95 6.95 ms; the paint leg still awaits a live game session. |
| G4 paint rate | **Met**: 60.1 fps mean, p50 60.0, p95 61.0, min 59.9 over 73.6 s with 20 boxes; paint 0.09 ms p50, 0.10 ms p95. Capture idle, animated window behind the overlay. |
| G5 soak | **Met for the capture path**: ten minutes, RSS +4.0 MB, 0 overwritten frames, 0 errors. Lost clicks and overlay paint drops need the overlay over a game, so they await a user session. |
| G6 hotkey | Ctrl+Alt+O registered at startup, plus the button. Twenty manual toggles with the game focused await a user session. |
| G7 recall | The audit tool counts proposals, primary and vehicle boxes per recorded frame. The human count over 100 gameplay frames awaits a user session. |
| G8 Phase 2 exit | Replay runs at a fixed 20 Hz through the same pipeline. The 20 example class check awaits a user session. |

Nothing above is marked met without a number or a run behind it. The rows that
depend on a human in front of a game say so rather than passing quietly.

## The soak that measured the app's own clock

One command, from `apps/irecon/src-tauri`:

```text
cargo run --release --bin capture_soak -- --seconds 600
```

It drives the production capture clock: the same `Run`, `FrameSlot` and Graphics
Capture handler the live loop uses, full primary display, no crop, no preview
encoding, no SQLite, no model. A consumer thread drains the slot the way the
inference thread does, so frames are genuinely consumed rather than piling up.
It exits non-zero if G1, G2 or G5 miss. It needs a changing screen: Graphics
Capture only delivers frames the compositor produced, so a motionless desktop
reads near zero and says nothing about the capture path. A game, a video, or any
animation over the display will do.

Conditions for the recorded run: 2560 by 1440 at 143 Hz, backend graphics
capture, 15 ms update interval, 600.2 s wall clock, release build, motion from a
small topmost WPF window animating on the compositor (no game was running).

| Measure | Result |
| --- | --- |
| G1 rate | **59.8 fps** over the run, **59.8 fps** over the last 60 s, floor 55 |
| G2 cost | **p50 4.62 ms, p95 4.91 ms, max 9.61 ms** |
| G5 RSS | 72.3 MB at 2 s to 76.3 MB at 600 s, **growth +4.0 MB** (limit 50) |
| G5 frames | 35,884 delivered, **0 overwritten**, **0 errors**, 35,884 pickups |
| Frame interval | p50 16.6 ms, p95 17.6 ms |
| Capture to slot publish | p50 4.63 ms, p95 4.91 ms, max 9.61 ms |
| Capture to consumer pickup | p50 5.33 ms, p95 6.95 ms, max 12.13 ms |

RSS at the minute marks: 72.3, 72.9, 73.6, 74.2, 74.8, 75.4, 75.4, 75.7, 75.7,
75.7, 76.3 MB. The last five minutes move by 0.6 MB, so that is the buffer pool
finishing its warm-up rather than a leak.

The capture-to-publish and capture-to-pickup rows exclude the model and the
paint. With no model loaded there is nothing to publish but the frame itself, so
those rows are the floor of the pipeline, not G3.

## The first run missed, and why

A twenty second run on the same path reported 51.4 fps and RSS growth of 48 MB.
Both misses were in the capture path, and both were fixed rather than budgeted:

1. Every frame allocated a fresh 14.7 MB RGBA buffer and freed it. The Windows
   heap keeps those pages, so the working set climbed with every second of
   capture and would have crossed 50 MB long before ten minutes. Frames now carry
   a buffer that returns to a small pool when the last holder drops it, so steady
   state allocates nothing.
2. A 16 ms minimum update interval against 60 Hz content that updates every
   16.7 ms occasionally pushed a frame to the next vsync. That showed up as a
   33 ms p95 frame interval and a 51 fps average. The interval is 15 ms now: still
   roughly a 66 fps ceiling, without aliasing against 60 Hz sources.

After both fixes a thirty second run reported 59.5 fps, cost p95 3.51 ms and RSS
growth 0.3 MB, and the ten minute run above held that line. The 16 ms figure in
the earlier E1 and E2 notes describes the probe, not the shipped cap.

## The paint clock, measured

G4 asks for 55 fps with 20 boxes while capture is idle. Calibration mode
draws 20 boxes that drift on a sine wave over whatever sits behind the
overlay, and the painter reports its own rate and draw cost once a second.

Conditions: 2560 by 1440 at 143 Hz, release binary from
`npm run tauri build -- --no-bundle`, capture idle, the overlay in calibration
mode over a small topmost WPF window animating on the compositor. 73.6 s of
samples, the first five dropped because the calibration event lands just
before the first painted frame.

| Measure | Result |
| --- | --- |
| Paint rate | **60.1 fps mean**, p50 60.0, p95 61.0, min 59.9, max 61.0 |
| Draw cost | mean 0.09 ms, p50 0.09, p95 0.10, max 0.11 |
| Boxes | 20 in every steady sample, calibration flag set in every sample |

That clears the 55 fps target with room: the paint clock follows the
compositor at 60 Hz on this display and the canvas work is a tenth of a
millisecond, so the overlay is not the leg that costs latency. The paint leg
of G3 over a running game still needs the play session.

Reproduce, from `apps/irecon`:

```text
npm run tauri build -- --no-bundle
IRECON_PAINT_LOG=<absolute csv path> IRECON_PAINT_CALIBRATION=1 IRECON_PAINT_SECONDS=75 \
  src-tauri/target/release/irecon.exe
```

In PowerShell set the three variables with `$env:` and start the exe. The
process shows the overlay after 1.5 s, switches calibration on 1.2 s later
and exits on its own after the requested duration. The CSV is
`epoch_ms,fps,draw_ms,boxes,calibration`.
Measuring twice exposed a race in the harness itself. The first run drew
20 boxes and the next drew none on the same code, because the calibration
event could land before the overlay page had registered its listeners, and
a fixed 1.2 s delay only made that less likely. The harness now waits for
the overlay's first painted frame, then sets the flag and emits, so the
mode is deterministic rather than probable.

A trap found while measuring: a plain `cargo build --release` embeds the dev
URL in the app context, so its windows load `http://localhost:1420` and stay
blank whenever the Vite server is not running. The overlay reported itself
visible while its page had never executed, which reads exactly like a paint
clock that reports nothing. App release binaries come from the Tauri CLI; the
soak harness is a plain cargo binary and is unaffected.

## Definition of done

Phase 0: G1 to G7 have numbers and the overlay sat over the game for ten minutes
without eating a click or leaking. Phase 1: a project survives a restart with
sources, classes and examples intact. Phase 2: G8 passes live and on replay
through the same code path. Then Phase 3 opens.
