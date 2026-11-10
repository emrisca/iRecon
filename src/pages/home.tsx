import { PageHeader } from "../components/page-header";
import { Button } from "../components/ui/button";
import type { Section } from "../components/app-sidebar";

const steps = [
  "Create a project on Projects. One project per game or app.",
  "Add a source on Sources: your display, a window, or a folder to replay.",
  "Grab or import frames on Library.",
  "Draw a box on Label, name the class, and save it.",
  "Start capture in Capture lab. Raise the match floor to drop weak boxes.",
  "Review uncertain boxes on Review. Accept or reject teaches the matcher.",
  "Record a session when you want to compare changes without the game.",
];

export function HomePage({ onSelect }: { onSelect: (section: Section) => void }) {
  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Start here"
        description="iRecon finds objects by example. Draw a few boxes, then match them live over a game or a replay. Everything stays on this machine."
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
          <div className="mt-2.5 flex flex-wrap items-center gap-2 border-t border-border pt-2.5">
            <p className="max-w-[65ch] text-xs text-muted-foreground">
              Frames, examples and recordings stay in the project folder. Use borderless
              windowed mode: an exclusive fullscreen game hides the overlay.
            </p>
            <Button
              size="sm"
              variant="outline"
              className="ml-auto"
              onClick={() => onSelect("projects")}
            >
              Open Projects
            </Button>
          </div>
        </section>
      </div>
    </div>
  );
}
