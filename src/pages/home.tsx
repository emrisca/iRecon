import { PageHeader } from "../components/page-header";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import type { Section } from "../components/app-sidebar";

const tabs: Array<{ id: Section; name: string; description: string }> = [
  {
    id: "projects",
    name: "Projects",
    description:
      "One project per game or app. Each keeps its own screens, classes, examples and recordings, so two projects never mix.",
  },
  {
    id: "sources",
    name: "Sources",
    description:
      "Choose where frames come from: a display, a window, an image or a folder. A crop keeps only the part of the frame the project cares about.",
  },
  {
    id: "library",
    name: "Library",
    description:
      "Every frame the project can label, whether it was grabbed from the screen or imported from disk.",
  },
  {
    id: "label",
    name: "Label",
    description:
      "Draw a box around an object, name its class, and save it as a positive or a negative example. This is what teaches the matcher.",
  },
  {
    id: "review",
    name: "Review",
    description:
      "Proposals that landed just under their class threshold, waiting for accept or reject. Every answer becomes an example.",
  },
  {
    id: "capture",
    name: "Capture lab",
    description:
      "Run live capture, read frames per second and inference cost, set the match floor, and control the overlay and detector.",
  },
  {
    id: "sessions",
    name: "Sessions",
    description:
      "Recorded capture with the proposals kept per frame. Replay it, re-score it with the current pool, or audit the detector.",
  },
  {
    id: "detectors",
    name: "Detectors",
    description:
      "Import a project-trained ONNX model, check its contract, and compare it with the shipped detector on a recording.",
  },
  {
    id: "eval",
    name: "Evaluate",
    description:
      "Mark what each frame actually contains and measure recall against a set you did not train on.",
  },
];

const steps = [
  "Create a project on Projects. One project per game or app.",
  "Add a source on Sources: the display you play on, or a folder of screenshots to replay.",
  "Grab or import frames on Library so there is something to label.",
  "Open Label, draw a box around the thing you want, name the class, and save it. A few positives and a few negatives are enough to start.",
  "Open Capture lab, pick the project, and press Start capture. Watch the boxes before the numbers.",
  "If boxes sit below the score you trust, raise the match floor in Capture lab. It drops weak boxes and weak review candidates.",
  "Review what the matcher was unsure about, accept or reject it, and the next run uses the answer.",
  "Record a session in Capture lab when you want to compare changes without launching the game again.",
];

export function HomePage({ onSelect }: { onSelect: (section: Section) => void }) {
  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Start here"
        description="iRecon finds game objects by example: you draw a few boxes, it matches them live over the game and over replays. Everything stays on this machine."
      />

      <div className="flex flex-col gap-2.5 p-2.5">
        <section className="rounded-lg border border-border bg-muted p-2.5">
          <h2 className="text-sm font-medium">Your first run</h2>
          <ol className="mt-1.5 flex flex-col gap-2">
            {steps.map((step, index) => (
              <li key={step} className="flex items-start gap-2 text-sm">
                <span className="w-5 shrink-0 text-xs text-muted-foreground tabular-nums">
                  {index + 1}
                </span>
                <span className="max-w-[65ch]">{step}</span>
              </li>
            ))}
          </ol>
        </section>

        <section className="flex flex-col gap-1.5">
          <h2 className="text-sm font-medium">The tabs</h2>
          <ul className="overflow-hidden rounded-lg border border-border">
            {tabs.map((tab, index) => (
              <li key={tab.id} className={index > 0 ? "border-t border-border" : undefined}>
                <div className="flex items-start gap-2 p-2">
                  <div className="flex min-w-0 flex-1 flex-col gap-1">
                    <span className="text-sm font-medium">{tab.name}</span>
                    <span className="max-w-[65ch] text-xs text-muted-foreground">
                      {tab.description}
                    </span>
                  </div>
                  <Button size="sm" variant="ghost" onClick={() => onSelect(tab.id)}>
                    Open
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        </section>

        <section className="flex flex-wrap items-center gap-2 rounded-lg border border-border p-2.5 text-xs text-muted-foreground">
          <Badge variant="outline">local only</Badge>
          <span>Frames, examples and recordings stay in the project folder.</span>
          <Badge variant="outline">borderless windowed</Badge>
          <span>
            An exclusive fullscreen game owns the display, so the overlay cannot appear over it.
          </span>
        </section>
      </div>
    </div>
  );
}
