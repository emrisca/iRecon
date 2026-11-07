# Decisions

Each entry states the choice, what it beat, and the condition that would reopen it. If an entry looks arbitrary in six months, the reason is here.

## 1. Tauri v2, not Electron

Electron ships a Chromium and a Node runtime per app, roughly 120 MB before any model. Tauri uses the system webview and a Rust core, which also means capture and inference run in the same process as the UI without an IPC bridge to a second runtime.

Reopen if: the webview's canvas cannot repaint 30 times a second with twenty boxes on any target machine, or a platform webview blocks transparent windows.

## 2. React and shadcn/ui

shadcn components are copied into the repository rather than installed, so the UI can be changed without fighting a library, and the project inherits the spacing, colour and icon rules used elsewhere. Tailwind keeps the theme tokens in one file, which is what makes light and dark both correct rather than one being an afterthought.

Reopen if: the component count needed for the review screen makes vendoring unmanageable, which is unlikely at this size.

## 3. ONNX Runtime through `ort`, not the OpenCV crate

OpenCV would add a native library to install, link and ship on two platforms, for operations that pure Rust crates already cover. The binding also describes its own API as unstable. `ort` downloads a prebuilt runtime, runs CPU plus CoreML or DirectML, and loads any ONNX model, which keeps the model choice independent of the app.

Reopen if: a feature needs calibration, optical flow, or homography, none of which are in the plan.

## 4. Two stages: proposals then identity, not one trained model

Training a detector per project means a labelling tool, a dataset format, a training run and a wait, which is the exact cost this app exists to remove. A shipped detector supplies proposals for free; the project only has to answer "is this one of mine", and that answer is a vector comparison that updates the instant an example is added.

Reopen if: proposal quality on the target games is so poor that the matcher rarely sees the object, which pushes custom detectors from Phase 5 to Phase 2.

## 5. Embedding similarity rather than template or keypoint matching

Template matching breaks on scale, rotation and lighting, which is every frame of a driving game. ORB and SIFT are more tolerant but still fail on motion blur and on objects that change shape between views. A learned encoder is trained for exactly this invariance, and one encoder handles every class in every project.

The exception is deliberate: fixed HUD elements still use normalized cross-correlation, because there the assumptions hold and a model would be waste.

Reopen if: a target object is so visually distinctive and so rigidly presented that a template would do, in which case it becomes a HUD mode, not a replacement.

## 6. One SQLite file per project, brute-force matching

A project is a folder a user can move, back up or delete. Per-project files make that true and make cross-project contamination impossible. Brute-force cosine over a few thousand vectors costs a fraction of a millisecond, so an index would be complexity with no measurable gain.

Reopen if: a project passes roughly 50k examples, at which point `sqlite-vec` is a drop-in.

## 7. Normalized coordinates across IPC

Pixels mean different things on different monitors, at different DPI, and in windows that resize. Sending `0..1` boxes and letting the overlay scale them removes a whole family of bugs and keeps recorded detections valid when replayed on a different display.

Reopen if: subpixel accuracy at 4K matters enough that float32 rounding shows, which it does not for rectangles.

## 8. Overlay as a separate window, capture exclusion on

A separate window can be positioned, made click-through, and excluded from capture independently of the app. Excluding it stops the overlay from feeding its own boxes back into the detector, which otherwise produces self-reinforcing boxes that grow.

Reopen if: exclusion is unavailable on a platform, which turns the fallback into subtracting last frame's boxes.

## 9. Borderless windowed is a requirement, not a limitation to engineer around

Exclusive fullscreen games own the display and overlays drawn by other processes do not compose over them reliably on either platform. Every commercial overlay that appears over exclusive fullscreen does it by hooking the game's swapchain, which means injection, which is off the table here.

State the requirement in the app. Do not build a hooking path.

Reopen if: the platform gains a supported composition path for exclusive fullscreen.

## 10. Local only, no account

The user's screenshots are game footage or their own desktop. Uploading them needs a privacy story, a backend, and a reason, and none of those exist for a tool whose value is immediate feedback.

Reopen if: sharing a project's examples becomes a real request, and even then it should be an export the user chooses to move.

## 11. RF-DETR Nano as the shipped detector

Ultralytics models are AGPL-3.0, which turns a distributed binary into a licensing event. RF-DETR Nano is Apache 2.0, exports to ONNX, needs no NMS, and reports 48.4 AP against YOLOX Nano's 25.8. YOLOX was the other permissive option and lost on recall, which is the one thing the proposal stage cannot afford.

The published latencies are GPU with TensorRT, so the CPU number is measured in Phase 0 before anything depends on it. Details in [models](05-models.md).

Reopen if: Nano at 384 boxes under half the cars in real footage, which promotes RF-DETR Small at 512 or a project-trained detector.

## 12. Automation last

Detections are already a clean event stream, so nothing in the plan blocks automation. Building rules before the detector is stable would mean a rules layer whose false alarm rate is inherited from flickering boxes, which is the fastest way to make the tool untrustworthy.

Reopen after Phase 4 exit criteria are met.
