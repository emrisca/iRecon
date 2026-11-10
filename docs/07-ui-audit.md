# UI audit: 0.4.0

This audit applies the ui-design, ste100 and unslop skills in this repository. The
app is a dense desktop tool. One person labels a few frames, then runs capture
against a game. Each screen must show the next action, the current state and the
evidence, in that order.

## Scope

The shell and nine screens: Projects, Sources, Library, Label, Review, Capture
lab, Sessions, Detectors and Evaluate. The audit used the running Vite view at
976 px and 1280 px, the source tree, and both colour modes.

## What changed

1. **Label had a permanent skeleton.** When no frame was open, the frame area
   showed a loading skeleton forever. It now shows a muted empty panel with the
   Grab frame action.
2. **Capture lab had the same permanent skeleton.** The latest frame area now
   says no frame exists and names the action that creates one.
3. **Projects used aria-labels for name fields.** The create and rename forms
   now have visible Name labels with matching `htmlFor` and input ids.
4. **Label merge had no visible label.** The class merge control now has a
   visible Merge into label and a select id.
5. **Sessions and Detectors used native confirm dialogs.** Both now show an
   inline confirmation bar with Delete and Keep. The file or recording is not
   removed until the user confirms in the page.
6. **Loading text sat inside buttons.** Review and Label now show a spinner and
   the text Loading, with `aria-busy` set. The control stays disabled.
7. **Evaluate used a text check mark.** The selected class state now uses the
   lucide Check icon beside the class name, so the label does not depend on the
   glyph or colour alone.
8. **Detectors empty states were bare paragraphs.** Project models and Project
   classifier now use muted panels with an icon and the next action.
9. **Settings showed a stale version fallback.** The panel now waits for the
   desktop app version instead of printing 0.3.0 in a browser preview.
10. **Label action hierarchy was checked.** Grab frame is the default action.
    Export dataset stays outline. The empty frame state points to Grab frame.

## What remains

- Native `select` controls remain in the sidebar, the Label merge control and
  the Detectors comparison form. They have visible labels and full keyboard
  behaviour, but they are not the shadcn/ui Select. Replacing them adds a Radix
  overlay layer to a dense desktop tool, so this pass did not change them.
- The browser preview cannot exercise screen capture, file pickers or the
  overlay. Those desktop-only states need a visual pass in the Tauri window.
- The window minimum is 960 px, so the 320 px reflow check does not apply. The
  976 px layout was checked.

## Out of scope

- No new screens, routes or backend commands.
- No change to capture, matching, sessions or detector behaviour.
- No new component library.
