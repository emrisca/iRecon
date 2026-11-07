# Models

Decision: **RF-DETR Nano** is the shipped proposal detector, exported to ONNX at 384 by 384. Apache 2.0, so it can ship inside a closed binary with no licence fee and no source disclosure.

## Why this one

| Model | Licence | Input | Params | COCO AP 50:95 | ONNX |
| --- | --- | --- | --- | --- | --- |
| RF-DETR Nano | Apache 2.0 | 384 | 30.5M | 48.4 | Yes, `model.export(format="onnx")` |
| RF-DETR Small | Apache 2.0 | 512 | 32.1M | 53.0 | Yes, same path |
| YOLOX Nano | Apache 2.0 | 416 | 0.91M | 25.8 mAP | Yes, weights published |
| YOLO11n | AGPL-3.0 | 640 | 2.6M | 39.5 | Yes, but wrong licence for a closed app |
| RT-DETR (Paddle) | Apache 2.0 | 640 | 20M+ | 45 to 53 | Exportable, less turnkey |

The three that matter:

- **YOLO11n is excluded on licence.** It is the obvious default in every tutorial, and it is AGPL-3.0. Shipping a binary means either open-sourcing the app or buying a commercial licence. Deciding this later means re-testing detection quality against a different model after the matcher is tuned around one, so it is decided now.
- **YOLOX is Apache 2.0 but old.** Its nano model is 0.91M params and 25.8 mAP, from 2021. Small enough to be tempting, but the proposal stage is the one place where recall matters more than speed: a car the detector never proposes is a car the matcher can never find. Half the accuracy of RF-DETR Nano is too much to give up before measuring anything.
- **RF-DETR is a DETR.** No anchors, no NMS in postprocessing, a fixed set of queries with scores. Fewer moving parts than a YOLO head plus NMS, and its input is 384 square, which is cheaper than 640 on a CPU. The cost is a DINOv2 transformer backbone, which is heavier per pixel than a small CNN. That is exactly what Phase 0 measures.

RF-DETR XLarge and 2XLarge use a different licence (PML 1.0), and Atto, Femto and Pico ship under that same restricted licence. Nano through Large is the Apache set. Do not drift upward by size without re-checking.

## Latency, honestly

The published 2.3 ms is a T4 GPU with TensorRT FP16. It says nothing about this app on a laptop CPU through ONNX Runtime. The realistic plan:

| Configuration | Expectation | Where it runs |
| --- | --- | --- |
| Nano at 384, CPU | 15 to 40 ms | Every machine, fallback |
| Nano at 384, CoreML | 5 to 15 ms | Apple silicon |
| Nano at 384, DirectML | 8 to 25 ms | Windows with any DX12 GPU |
| Small at 512, CPU | 30 to 70 ms | Only if Nano misses distant objects |

These are ranges to confirm, not measurements. Phase 0 question B produces the real number on the target machine, and the architecture's 30 fps target depends on it.

## Interface contract

The exporter's output order is the thing that breaks integrations, so pin it with a test that runs on a known image and compares against a saved result.

- Input: `1 x 3 x 384 x 384`, float32, RGB, letterboxed with the padding colour the exporter was trained with.
- Output: boxes as normalized centre `cx cy w h`, class scores, and labels. Filter at 0.3, keep the top 100, convert to corners, clamp, then scale back through the letterbox.
- Preprocessing constants live in one Rust module with a test, because a wrong mean or a swapped channel order produces boxes that look plausible and are subtly wrong.

If the exported graph expects a different resolution, take it from the session's input shape at load time rather than hardcoding 384.

## Where the file lives

`models/rf-detr-nano.onnx` under the app data directory, downloaded on first run only after a visible prompt, with the expected SHA-256 recorded in the app and checked on load. A model that fails the hash is refused rather than run.

Bundling it into the installer is the alternative. It makes the download disappear and the installer about 120 MB larger. Start with the download, revisit at packaging time.

## The second model

The encoder that turns crops into vectors is a separate decision, and a smaller one:

| Candidate | Licence | Note |
| --- | --- | --- |
| SigLIP base, image tower only | Apache 2.0 | Good default, 224 input, ships as ONNX |
| CLIP ViT-B/32 | MIT | Smaller and faster, weaker on fine visual differences |
| DINOv2 | Apache 2.0 code, check the weights | Strong features, but the licence covers code and weights separately |
| MobileCLIP | Apple sample licence | Fast, but the terms need reading before it ships |

Prefer SigLIP base first. If crop matching turns out to be the bottleneck on CPU, CLIP ViT-B/32 at 224 is the cheaper experiment. Either way the encoder is versioned per project, so swapping it re-embeds examples in the background instead of corrupting the pool.

## What Phase 0 must answer

Run the exported Nano model over 100 frames of recorded Assetto Corsa traffic at 384 and 512, on the machine that will run the app, and record:

1. How many visible NPC cars get a box, and how many boxes are not cars.
2. Milliseconds per frame on CPU, and on the GPU execution provider if one is available.
3. Whether a distant car, the case that matters most for traffic awareness, survives the 384 input or needs 512.

If Nano at 384 boxes under half the cars, move to Small at 512 before touching the UI. If both fail, the proposal stage needs a project-trained detector and Phase 5 moves up.
