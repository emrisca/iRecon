# iRecon

A desktop app that learns what you point it at, then boxes those things on your screen while you play or work.

You give it a handful of screenshots of the objects you care about: the rival cars in a racing game, a character on screen, a widget in an app. It finds them in live screen capture and draws a box around each one, with a confidence score. Everything stays on your machine. No upload, no account, no per-image billing.

Status: spike running. Capture, the transparent overlay and RF-DETR Nano detection work end to end against a real screen, and the labeling page turns a grabbed frame into a named example. Sample evaluation, the crop matcher and project-trained detectors are still ahead; see [the roadmap](docs/03-roadmap.md).

## The loop

1. Create a project. One project per game or app, each with its own database and its own examples.
2. Point the project at a monitor, a window, or a video file. Draw a crop region if you only care about part of the frame.
3. Add examples: crop a car, name it, repeat. Ten to thirty positives is enough to start, plus a few negatives.
4. Press start. A detector proposes boxes, the project's matcher scores each one, and the overlay draws the ones above threshold.
5. Review what it found between runs. Accept or reject each candidate. Every answer becomes another example, and the next run is better.

Step 5 is the part most tools leave to you. Here it is the main screen.

## How it works

Two stages, because one model cannot do both jobs well.

**Stage one, proposals.** RF-DETR Nano, Apache 2.0 and shipped with the app, finds everything that looks like a discrete object in the frame. It does not need to know your object. It only needs to say "something is here, look at this crop". This is what makes a handful of examples enough: you never train a detector from zero.

**Stage two, identity.** Each proposal crop is passed through an image encoder that turns it into a vector. The project stores vectors for your examples, and a crop is a match when it lands close enough to one of them. A new example is a new row, not a training run, so adding one takes effect immediately.

The third matcher is narrow and exact: template matching for fixed HUD elements, where scale and rotation never change. Health bars, minimap markers, icons. Cheap, and it works.

Fine-tuning a detector is a later step, for objects stage one refuses to propose at all. It is an offline job with a dataset export, not something the live loop waits for.

## What this is not

- Not a general model trainer. It optimises for "good enough in two minutes", not leaderboard accuracy.
- Not memory reading and not injection. It sees pixels, the same pixels a screenshot would capture. Nothing is written into another process.
- Not a bot. Detections are published as an event stream so a rules layer can be added later, but the app itself only draws boxes.

## Stack

Tauri v2 shell, React and TypeScript front end, shadcn/ui components and Tailwind, Rust core for capture, inference and storage, ONNX Runtime for models, SQLite per project. No OpenCV dependency; see [the architecture note](docs/02-architecture.md#why-not-opencv) for why.

## Running it

Needs Node 20 or newer, a Rust toolchain, and the platform build tools Tauri expects: Xcode command line tools on macOS, MSVC build tools and WebView2 on Windows.

```bash
npm install
npm run tauri dev
```

The first build compiles the Rust side and takes a minute or two. After that, front end edits reload instantly and Rust edits recompile.

On macOS the first capture run asks for Screen Recording. Grant it to iRecon, then restart the app, or the preview stays black. Windows asks for nothing. A game in exclusive fullscreen is the case that will not work, so run it in borderless windowed.

`npm run build` typechecks and builds the front end without opening the desktop window.

The detector is not in the repository. The Capture lab page downloads it on request, 108 MB once, and checks it against a pinned SHA-256 before anything runs. To fetch it without opening the app:

```bash
cd src-tauri
cargo run --example fetch_model
cargo run --example detect_image -- path/to/photo.jpg
```

`cargo test` covers preprocessing and the ONNX input and output contract. The contract test skips itself with a message when the model file is not downloaded yet, so a fresh clone still tests clean.

## Layout

```
docs/01-product.md        what it does, who for, features by phase
docs/02-architecture.md   pipeline, crates, data model, IPC, budgets
docs/03-roadmap.md        phases, spike checklist, risks
docs/04-decisions.md      decisions and what would reverse them
docs/05-models.md          which detector, why that licence, what it must prove
```

## Documents

Start with the product note for the idea, then the architecture note before writing code. The roadmap is the order of work, and the decisions file is where to look when a choice looks arbitrary later.
