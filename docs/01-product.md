# Product

## The problem

Training a detector normally means a labelling tool, a dataset format, a training run, a GPU, and a wait. That is the right cost for a production model and the wrong cost for "I want to know when a police car is behind me in the game I am playing".

The gap is not accuracy. It is the distance between having twenty screenshots and having something that runs on your screen. iRecon is an attempt to make that distance one afternoon, and to make improving it a by-product of using it.

## Who it is for

**The traffic watcher.** Plays a driving game with AI traffic. Wants boxes on the NPC cars, and eventually a warning when something is closing from behind. Cannot label a dataset; can drag a box around a car.

**The character tracker.** Plays something with distinctive characters and wants to know where they are on screen, for a stream overlay or for their own curiosity. The detector probably does not know that character, so the matcher does most of the work.

**The screen watcher.** Not a game. A dashboard, a queue, a log viewer. Wants a box when a row appears that matches a screenshot they saved. Fixed layout, so template matching is enough.

The common shape: few examples, immediate feedback, local only, no willingness to learn a tool.

## The loop in detail

A **project** owns everything: sources, classes, examples, sessions, models, thresholds. Switching project switches database and example pool, so a traffic project and a HUD project never contaminate each other.

A **source** is where frames come from: a display, a window, a video file, or a folder of images. The file and folder kinds exist for development and for replay, and they are the reason the pipeline can be tested without launching a game.

A **class** is a thing you want boxed: `rival-car`, `police`, `cone`. Each has a colour, a threshold, and a pile of examples. A class needs positives and negatives. Negatives are background, other cars, the road, anything that shows up often and is not the target. Without them the matcher learns "car-shaped", not "this car".

An **example** is a crop plus a label. It is a positive when the crop contains the class, a negative when it does not. Examples arrive three ways: imported from files, drawn over a frame inside the app, or harvested from a session.

A **session** is a run against a live source: start, capture, detect, box, stop. Sessions record their frames and detections so a run can be inspected later.

A **candidate** is a proposal the matcher was unsure about, saved for review. This is the teaching surface. The review screen shows a candidate with its score and its nearest examples, and three buttons: this is the class, this is not, skip. Accepting writes a new positive, rejecting writes a negative, and both re-rank the next session immediately.

The review screen is also where the confidence problem gets solved. Scores are only meaningful relative to your own pool, so the app should show the score distribution per class rather than a bare percentage, and let the threshold be set by dragging a line through that distribution.

## The three matchers

| Matcher | Works on | Strength | Weakness | Use for |
| --- | --- | --- | --- | --- |
| Detector proposals | Any object with edges and texture | Finds things anywhere, at any scale | Its class list is fixed; unknown classes come back as the nearest known box or not at all | Stage one, always on |
| Embedding similarity | Crops | A new example works instantly; robust to lighting, angle, mild scale change | Needs proposals to look at; a badly cropped box embeds badly | The per-project matcher |
| Template matching | Fixed HUD regions | Exact, microseconds, no model | Breaks on scale, rotation, occlusion, animation | Icons, bars, fixed UI |

The ordering matters. Template matching on full frames is what people try first, and it is the thing that disappoints them, because a car at forty metres shares almost no pixels with the same car at five. The embedding stage exists to survive that.

## Features by phase

**Phase 1, the shell.** Projects list, project creation, source picker with layout preview, crop region, capture library import, class list, example grid, light and dark.

**Phase 2, the live loop.** Capture thread, proposal detector, encoder, matcher, overlay with boxes and labels, per class toggle, threshold slider, FPS and latency readout.

**Phase 3, the teaching loop.** Candidate harvesting with a similarity floor, review queue with nearest neighbour evidence, accept and reject feeding the pool, coverage hints ("this class has three examples, all from behind").

**Phase 4, stability.** Track IDs across frames, box smoothing, a dwell rule so a box appears after N frames rather than flickering, session recording and replay, per class statistics.

**Phase 5, custom detectors and export.** Export a project as a YOLO dataset, train a small detector outside the app, import the ONNX, and let a project swap stage one for its own detector. This is the answer for objects the shipped detector will not propose.

**Phase 6, the automation substrate.** The event stream becomes a documented contract with tracks, timestamps and zones. A rules layer subscribes: "when a track labelled rival-car enters this zone within N metres, emit an event". Deliberately last, because a rules layer over an unstable detector is a bug generator.

## Ideas worth keeping

Not scheduled, but they follow from the design and should not be blocked by it.

1. **Nearest example evidence.** Every detection can answer "which of your screenshots made me say that". It is one lookup and it makes the model legible, which is what makes people trust the review screen.
2. **Zone triggers.** A drawn rectangle that only reports detections inside it. Cheaper than filtering later, and it is the natural UI for "behind me" in a driving game.
3. **Relative size as distance.** Box height in a driving game is a crude but usable distance proxy. One scalar per track, no extra model.
4. **Stability over confidence.** Showing a box only after it persists four frames removes most flicker and looks far more accurate than a higher threshold does.
5. **Session replay.** A recorded session replays through the same pipeline at 1x or 10x. Tuning without launching the game is the difference between a usable tool and a chore.
6. **HUD mode.** A separate pipeline that runs template matching on declared fixed regions, displayed in the review screen next to the detector's output so it is obvious which one is doing the work.
7. **Grouped classes.** `traffic` containing `car`, `truck`, `bus`, with one threshold and one colour. Saves the user from managing three near-identical classes.
8. **Dataset export is a feature, not a debug tool.** Everything the app knows, in a format another tool can read, is what stops the project from being a dead end.
9. **Import from another project.** Same game, new season, new car pack: start from the previous examples rather than zero.
10. **Explain the threshold.** On a new class, show the score histogram and suggest a cut rather than defaulting to 0.75 and letting the user think the model is broken.

## Naming

Use these words in UI, code and docs. They are the whole vocabulary.

| Term | Means |
| --- | --- |
| Project | One game or app, one database, one example pool |
| Source | Where frames come from: display, window, file, folder |
| Class | A thing to box, with colour, threshold, examples |
| Example | A labelled crop, positive or negative |
| Candidate | An unconfirmed detection waiting for review |
| Session | One live run, recorded for replay |
| Track | One object followed across frames |

## Open questions

1. Is the shipped detector good enough for the examples that matter most (racing games, stylised characters), or does the first real project need a custom detector on day one? Phase 0 answers this with real frames.
2. Does the review screen need to show motion, or is a still crop enough to judge a candidate?
3. How does a user describe a negative that is "nearly right"? A rejected candidate is a negative, but a box that is 80 percent right and 20 percent background teaches the matcher the background.
4. Is a per-class threshold the right control, or does the app pick it and hide it?
5. How much of the frame should the detector see? Full display, the crop region, or a downscaled letterbox with a minimum object size in pixels.
