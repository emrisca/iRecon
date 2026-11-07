# Roadmap

The order is chosen so that the things which can invalidate the idea are tested first, on real frames, before any UI is built around them.

## Phase 0: the spike

Time box: two days. Throwaway code is fine. Two questions, one machine each.

**Question A, can we draw over the game?** On the Windows machine, in Assetto Corsa, borderless windowed, 1440p or whatever the display is.

- Tauri v2 shell, one button.
- Capture the primary display through `scap`.
- Overlay window, transparent, click through, always on top, sized to the display.
- Draw three fake rectangles that move in a sine wave.
- Toggle the overlay with a hotkey.
- Status line: capture fps, frames dropped, overlay paint time.

Acceptance: overlay visible over the game for ten minutes, boxes where the code says they are, no leak, capture above 50 fps with the game running, no click eaten by the overlay.

**Question B, is the shipped detector useful on this footage?** Offline, no Tauri.

- Grab a hundred frames from a recorded session of Assetto Corsa traffic.
- Run RF-DETR Nano on them at 384, then at 512 with Small.
- Count: boxes on cars, boxes on things that are not cars, cars with no box.

Acceptance: at least 80 percent of visible NPC cars get a box, and false positives are detached blobs that a matcher can reject rather than merged into one giant box. If this fails, the project needs a custom detector and the plan changes before Phase 2.

Do these before building any screen. If either fails, the product idea needs a different shape and it is much cheaper to learn that now.

## Phase 1: the shell

- Tauri v2 scaffold, React, TypeScript, Tailwind, shadcn/ui, light and dark, project list.
- Project create, open, delete, with the on-disk layout and SQLite migrations.
- Source picker: displays, windows, file, folder. Preview of one frame with the crop rectangle.
- Capture library: import images, bulk import a folder, thumbnail grid.
- Classes: create, colour, rename, delete, example count.
- Example editor: open a frame, drag a box, assign a class, save the crop.
- Example grid per class with positive and negative separation.

Exit: a project can be created, filled with examples by hand, and reopened after a restart with everything intact.

## Phase 2: the live loop

- Capture thread and ring buffer, engine thread, overlay window.
- RF-DETR proposals, score filter, box size floor. No NMS: the model returns a fixed query set.
- Encoder, project pool load, matching, per class threshold.
- Overlay canvas with boxes, labels, confidence, per class visibility.
- Status bar: fps, inference time, device, dropped frames.
- Sources file and folder run through the same pipeline at a fixed rate.

Exit: a class with twenty examples boxes the right things in a recorded session at 20 fps or better, and a class with no examples produces no boxes.

## Phase 3: the teaching loop

- Candidate harvesting: proposals whose best score sits in a band around the threshold, saved to disk with a cap per session.
- Review queue: candidate, score, nearest three examples side by side.
- Accept, reject, skip. Accepted becomes a positive with the crop; rejected becomes a negative.
- Coverage hints: count per class, and a warning when all examples look alike (same angle, same size, same lighting) computed from embedding distance.
- Threshold suggestion from the score histogram of the current pool.

Exit: forty reviewed candidates measurably improve recall on a held-out recorded session, measured before and after.

## Phase 4: stability

- IoU tracker with ids, a dwell counter, and a miss counter.
- Box smoothing so a track does not jitter.
- Session recording: frames, detections jsonl, replay at 1x and 10x.
- Per session stats: detections per class, average confidence, track length.
- Replay runs the current pool against recorded frames, which is how tuning happens without launching the game.

Exit: boxes stop flickering, and a tuning change can be evaluated in under a minute from a recorded session.

## Phase 5: custom detectors

- Dataset export in YOLO format: images, labels, data yaml, class list, negatives.
- Import a trained ONNX as a project detector, with a shape check and a warmup test.
- A guided path for the training step outside the app, documented rather than automated, since installing PyTorch inside a desktop app is a support burden.
- Side-by-side comparison of shipped detector and project detector on a recorded session.

Exit: an object the shipped detector never proposes gets boxes after one training round.

## Phase 6: the automation substrate

- Public event contract: `track`, `class`, `zone`, `entered_at`, `left_at`, `frames_seen`.
- Zones drawn in the live view.
- A rules layer, shape to be decided then: built-in conditions, a small script format, or a plugin.
- The first concrete rule: a car approaching from behind inside a zone.

Not before Phase 4. Rules over a flickering detector produce false alarms, and false alarms are how a tool gets uninstalled.

## Deliberately deferred

Video file export of a session with boxes burned in. Cloud sync. Model sharing. Multi-user. Mobile. Linux capture. Anything that reads game memory. Anything that sends input to another process.

## Risks

| Risk | Impact | Mitigation | Tripwire |
| --- | --- | --- | --- |
| Overlay does not work over the game | Idea is dead on Windows | Phase 0 spike, borderless windowed required | Spike fails on two machines |
| Shipped detector does not propose the target objects | Matcher never sees them | Phase 0 question B, custom detector path in Phase 5 | Under 50 percent of visible cars boxed |
| Detector licence blocks distribution | Rewrite of the detection stage | Choose Apache-2.0 from the start if the app will ship closed | Any plan to publish binaries |
| Inference too slow on the target GPU | Boxes lag the game | 640 or 512 input, batch crops, drop small boxes, CoreML or DirectML | Under 15 fps on the target laptop |
| Matcher is fooled by similar shapes (all cars look alike) | Low precision, review queue floods | Negatives from other cars, margin scoring, per class threshold | Precision under 60 percent after 40 reviews |
| Scope creeps into automation | Never ships a loop | Phase 6 stays closed until Phase 4 exits | Any rule engine code before then |
| Disk fills with frames | App feels broken | Recording off by default, prune to last N sessions, show size | Project folder over 5 GB |
| Game terms of service forbid overlays | User gets banned | No injection, plain language warning, local only | Any request to read memory or send input |

## First three commits

1. Scaffold: Tauri v2, Vite, React, Tailwind, shadcn, light and dark, empty project list.
2. Storage: SQLite schema, migrations, project CRUD, on-disk layout.
3. Capture spike: one frame from a display to a preview, no overlay yet.

Small enough that each one is reviewable in a sitting.
