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

First measurement, development Mac, CPU execution provider, debug build: **170 ms per frame** at 384 on an 810 by 1080 frame, and the same image decoded to the right boxes (bus 0.96, three people above 0.92). That is far outside the 15 to 40 ms estimate and outside a 30 fps budget, but it is an unoptimised debug build on a cold session. Re-measure in release, then with the CoreML provider, before deciding whether Nano at 384 is the shipping configuration or whether the proposal stage needs to be cheaper.

## Interface contract

The exporter's output order is the thing that breaks integrations, so `detector.rs` pins it in `onnx_contract_is_pinned`, and the loader refuses a file whose input or output names do not match.

- Input: a tensor named `input`, `1 x 3 x 384 x 384`, float32, RGB, **plain stretch** to 384 square. Not letterboxed: the export was trained and exported this way, and padding here shifts every box.
- Normalisation: divide by 255, then ImageNet mean `[0.485, 0.456, 0.406]` and standard deviation `[0.229, 0.224, 0.225]`. Channel order is RGB, not BGR.
- Outputs, matched **by name**, never by shape or index: `dets` `1 x 300 x 4` normalised centre `cx cy w h`, and `labels` `1 x 300 x {classes}` **pre-sigmoid** logits. For the stock COCO export that is 91 slots, with slot 0 background. 300 queries, no NMS, so the set is already deduplicated.
- Postprocessing: sigmoid each slot that has a name, take the best one per query, drop below 0.35, convert to corners, clamp to the frame, keep the top 100.

A fine-tuned model usually has fewer slots than COCO. The loader probes the slot count from the graph and names each slot. Put a `<model>.labels.json` beside the ONNX file when the model does not use the stock COCO layout:

A wrong mean or a swapped channel order produces boxes that look plausible and are subtly wrong, so the constants and the preprocessing test live beside the decode in one module. Query count and class slots are probed from the loaded graph rather than assumed, and the test asserts 300 and 91 for this artifact.

JSON sidecars can be a plain array of class names, or a table with an explicit background slot:

```json
["Enemy", "Ally"]
```

```json
{ "names": ["Enemy", "Ally"], "background": -1 }
```

With an array, as many names as slots means there is no background slot, and one fewer name means there is one background slot: slot 0 for a 91 slot graph, the last slot for any other size. In the table form, `background` is `-1` for the last slot or a zero-based slot index. Without a sidecar, a 91 slot graph uses the COCO names, a one slot graph is a single `object` class, and any other size treats the last slot as background with numbered names. A sidecar is the only way to tell a two class model with no background apart from a one class model with a spare slot, so custom exports should ship one.

If a future export changes resolution or names, the loader fails loudly instead of silently misreading the output.

## Where the files live

Each model lands as its own file under `models/` in the app data directory, downloaded per row from the Models card on the Capture lab page. The expected SHA-256 is recorded in the app and checked again on load; a model that fails the hash is refused rather than run, and a partial download is deleted rather than resumed. The last model loaded is written to `models/active.txt`, so the next start loads the same size instead of guessing.

Both pins are unofficial Apache-2.0 conversions of the unmodified COCO weights, and both share the output contract below. They differ in input size, file size and speed:

| Model | Input | Bytes | SHA-256 |
| --- | --- | --- | --- |
| RF-DETR Nano | 384 | 107,835,234 | `21045851f3906c7d424874363fe933477fda7210bad0d7a69659fb98f8845890` |
| RF-DETR Small | 512 | 114,448,791 | `31fe724f8d5dbff5d09eb4c2c44d2d4083c086c167aec54e11e694e652307f9b` |

URLs follow the same shape: `https://huggingface.co/open-noodle/rfdetr-<size>/resolve/main/detection/model.onnx`.

Nano is the default because it is the cheapest proposal pass. Small costs more per frame and reads smaller objects, which is the trade to measure on the Windows machine before either becomes the shipping choice. Switching is one click, no reinstall, and the capture loop keeps running: the next frame uses the newly loaded graph.

Verified on load: input `input`, outputs `dets` and `labels`, 300 queries, 91 class slots. The input size is part of the catalog entry, not a constant in the decoder, because a 512 graph refuses a 384 tensor. The first artifact this project pinned went private, which is the reason each URL is a constant beside its hash rather than something fetched from a registry at runtime: a broken source is a one-line change, and nothing unverified is ever executed.

`cargo run --example fetch_model -- rfdetr-small` downloads and verifies without opening the app. `cargo run --example detect_image -- path/to.jpg path/to/model.onnx` prints boxes for one image, which is the quickest way to compare two exports before touching the UI.

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

Run the exported Nano model over 100 frames of recorded gameplay traffic at 384 and 512, on the machine that will run the app, and record:

1. How many visible NPC cars get a box, and how many boxes are not cars.
2. Milliseconds per frame on CPU, and on the GPU execution provider if one is available.
3. Whether a distant car, the case that matters most for traffic awareness, survives the 384 input or needs 512.

If Nano at 384 boxes under half the cars, move to Small at 512 before touching the UI. If both fail, the proposal stage needs a project-trained detector and Phase 5 moves up.

`cargo run --example batch_detect -- path/to/frames` produces the numbers. It verifies the pinned hash, runs every PNG and JPEG in the folder through the model, and writes `frames.csv` (size, boxes, car-like boxes, inference ms, top class), `boxes.csv` (every box with its score and normalised corners) and `classes.csv` (totals per class) into `irecon-batch`. Recall is the one number it cannot produce, because that needs a human count of the objects actually visible; the car-like column is the numerator for it, and the zero-box frames are where the misses are hiding.

## Stock YOLO exports

A project detector can also be a normal Ultralytics YOLOv8 or YOLO11 detection export. It must have one output shaped `[1, 4 + classes, anchors]` or the transposed `[1, anchors, 4 + classes]`, with box centre and size in input pixels and class scores beside them. iRecon letterboxes the frame to the graph's square input, decodes the best class per anchor, applies per-class NMS at 0.45 IoU, and maps boxes back to the frame. Class names come from `<model>.labels.json`; an 80 class export falls back to the COCO names, and anything else is labelled `class N`, which the matcher overwrites anyway. The two catalog models stay RF-DETR. YOLOv5 exports with a separate objectness score are not claimed: the extra channel shifts the class columns and would decode as a wrong class.
