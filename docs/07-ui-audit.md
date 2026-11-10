# UI audit: one shell, ten screens

This audit applies the ui-design, ste100 and unslop skills in this repository. The
app is a dense desktop tool used by one person who labels a few frames, then runs
capture against a game. The screen must show the next action, the current state
and the evidence, in that order.

## What the screens do

| Screen | Single job | Dominant element |
| --- | --- | --- |
| Start here | Get a new project to its first box | Numbered first run |
| Projects | Choose or create the project | Project list |
| Sources | Choose where frames come from | Source picker |
| Library | Find a frame to label | Frame grid |
| Label | Draw one box and save it | Frame stage |
| Review | Answer uncertain candidates | Candidate image |
| Capture lab | Start capture and read its cost | Live frame and metrics |
| Sessions | Replay and re-score a recording | Session viewer |
| Detectors | Compare a project model with the shipped one | Comparison table |
| Evaluate | Measure recall on held-out frames | Recall table |

## Findings

1. Ten destinations sit in one flat list. Setup, teaching and running are mixed,
   so a new user cannot tell what order to use.
2. The shell has no way to close the sidebar. The 224 px column is always
   reserved, even on a narrow window or while inspecting a frame.
3. Start here repeats the whole navigation as a second list with ten Open
   buttons. The sidebar already names and opens every screen.
4. Capture lab stacks eight metric cards, then repeats four of those numbers in
   a status line. It also carries a long card of limitations that does not help
   the current run.
5. Model rows in Capture lab are cards inside a card. Detectors uses raw inputs
   and two unlabelled selects, so the forms do not match the rest of the app.
6. Sessions offers a detector audit that reads downsampled previews and says so
   in its own help text. It reports a number that is not trustworthy.
7. Library, Sources, Label and Detectors use placeholders or aria-labels where a
   visible label belongs. Several small gaps are 2 or 4 px, below the 6/8/10
   scale.
8. Review and Evaluate use 10 px text. Evaluate uses an em dash as a value.

## Plan

### Shell

- Group navigation into Set up, Teach and Run, with Start here above them.
- Add a sidebar toggle to the page header, close and reopen the whole panel,
  persist the choice, and add Control+B.
- Remove the duplicate version badge and the repeated local-only footer line.
  Settings already shows the version and Start here explains the local promise.
- Keep one accent for the active item and the live state.

### Start here

- Keep the first-run sequence. Cut the duplicated tab catalog and its ten
  buttons.
- Keep the local-only and borderless facts as one compact note.

### Capture lab

- Replace the three stacked toolbars with one run panel: project, source,
  record and match floor.
- Keep four primary metrics: frames per second, inference, matcher and frame to
  box. Move interval, capture percentiles, preprocess, drops, resolution and
  backend into one status line.
- Remove the limitations card. Borderless mode stays in Start here.
- Flatten model rows into divided rows instead of nested cards.

### Screens with duplicated or untrustworthy sections

- Sessions: remove the detector audit action and its output. Keep re-scoring
  with the current pool, which measures the thing the screen is for.
- Start here: remove the tab catalog.
- Capture lab: remove the caveat card.

### Forms and type

- Give Library, Sources, Label and Detectors visible labels.
- Replace 10 px text with the 12 px meta size.
- Replace the em dash value in Evaluate with n/a.
- Raise every 2 or 4 px gap to 6 px, and keep 8 and 10 px for groups.
- Keep the system sans stack, semantic colour tokens and both colour modes.

## Out of scope

- No new screens, routes or backend commands.
- No changes to capture, matching, sessions or detector behaviour.
- No new component library. Existing shadcn/ui primitives stay in use.
