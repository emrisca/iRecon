import { useEffect, useRef, useState } from "react";
import {
  Crop,
  Download,
  Keyboard,
  Maximize2,
  Minimize2,
  Minus,
  Plus,
  RotateCcw,
  Trash2,
} from "lucide-react";

import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import { Input } from "../components/ui/input";
import { Skeleton } from "../components/ui/skeleton";
import { Separator } from "../components/ui/separator";
import { FrameStage } from "../components/frame-stage";
import { Pencil } from "lucide-react";
import { PageHeader } from "../components/page-header";
import { useProject } from "../components/project-context";
import { useSidebar } from "../components/sidebar-context";
import {
  CLASS_COLORS,
  classAdvice,
  cleanupClasses,
  createClass,
  createExample,
  deleteClass,
  deleteExample,
  deleteFrame,
  embedExamples,
  exampleImage,
  mergeClasses,
  exportDataset,
  frameImage,
  grabFrame,
  grabSettings,
  isDesktop,
  listClasses,
  listExamples,
  listFrames,
  listSources,
  matchRegion,
  onFrameGrabbed,
  onGrabError,
  openFrame,
  resetDemoData,
  setClassThreshold,
  setGrabShortcut as saveGrabShortcut,
  setGrabTarget,
  updateClass,
} from "../lib/api";
import type {
  Box,
  ClassAdvice,
  ClassMatch,
  DatasetExport,
  ExampleView,
  FrameView,
  LabelClass,
  SourceView,
} from "../lib/types";

const GRAB_SHORTCUT_KEY = "irecon.label.grabShortcut";
const DEFAULT_GRAB_SHORTCUT = "Ctrl+Alt+G";

function isFunctionShortcut(value: string) {
  return /^F([1-9]|1[0-2])$/.test(value.trim().toUpperCase());
}

function canonicalModifier(value: string) {
  switch (value.toLowerCase()) {
    case "ctrl":
    case "control":
      return "Ctrl";
    case "alt":
    case "option":
      return "Alt";
    case "shift":
      return "Shift";
    case "meta":
    case "super":
    case "cmd":
    case "command":
      return "Meta";
    default:
      return value;
  }
}

function shortcutLabel(key: string) {
  if (key === " ") return "Space";
  return key.length === 1 ? key.toUpperCase() : key;
}

/** A global bare key would block typing in every app, so it gains Ctrl+Alt. */
function normalizeGrabShortcut(value: string) {
  const parts = value
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts.length === 0) return DEFAULT_GRAB_SHORTCUT;
  const key = shortcutLabel(parts.pop() ?? "");
  if (parts.length === 0) {
    return isFunctionShortcut(key) ? key.toUpperCase() : `Ctrl+Alt+${key}`;
  }
  return [...parts.map(canonicalModifier), key].join("+");
}

function readGrabShortcut(): string | null {
  if (typeof window === "undefined") return DEFAULT_GRAB_SHORTCUT;
  const saved = window.localStorage.getItem(GRAB_SHORTCUT_KEY);
  if (saved === null) return DEFAULT_GRAB_SHORTCUT;
  if (saved === "none") return null;
  return normalizeGrabShortcut(saved);
}

function shortcutMatches(event: KeyboardEvent, shortcut: string) {
  const parts = shortcut
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);
  const rawKey = parts.pop();
  if (!rawKey) return false;
  const modifiers = new Set(parts.map((part) => part.toLowerCase()));
  const ctrl = modifiers.has("ctrl") || modifiers.has("control");
  const alt = modifiers.has("alt") || modifiers.has("option");
  const shift = modifiers.has("shift");
  const meta =
    modifiers.has("meta") ||
    modifiers.has("super") ||
    modifiers.has("cmd") ||
    modifiers.has("command");
  if (
    event.ctrlKey !== ctrl ||
    event.altKey !== alt ||
    event.shiftKey !== shift ||
    event.metaKey !== meta
  ) {
    return false;
  }
  const key = shortcutLabel(rawKey);
  if (key === "Space") return event.code === "Space";
  return event.key.toLowerCase() === key.toLowerCase();
}

type PendingBox = {
  id: string;
  box: Box;
  className: string;
  role: "positive" | "negative";
  color: string;
};

let pendingBoxId = 0;

function ExampleTile({ example }: { example: ExampleView }) {
  return (
    <li className="media-tile flex flex-col gap-1.5">
      <img
        src={exampleImage(example)}
        alt={`Example from ${example.createdAt}`}
        loading="lazy"
        decoding="async"
        className="aspect-square w-full rounded-md border border-border object-cover"
      />
      <span className="truncate text-xs text-muted-foreground tabular-nums">
        {example.width} by {example.height}
      </span>
    </li>
  );
}

const FRAME_PAGE = 400;
const EXAMPLES_PAGE = 400;

export function LabelPage() {
  const { projects, projectId, selectProject } = useProject();
  const { live } = useSidebar();
  const [classes, setClasses] = useState<LabelClass[]>([]);
  const [frame, setFrame] = useState<FrameView | null>(null);
  const [box, setBox] = useState<Box | null>(null);
  const [pending, setPending] = useState<PendingBox[]>([]);
  const [fullscreen, setFullscreen] = useState(false);
  const [className, setClassName] = useState("");
  const [selected, setSelected] = useState<LabelClass | null>(null);
  const [examples, setExamples] = useState<ExampleView[]>([]);
  const [examplesMore, setExamplesMore] = useState(false);
  const [examplesLoading, setExamplesLoading] = useState(false);
  const [matches, setMatches] = useState<ClassMatch[]>([]);
  const [exportInfo, setExportInfo] = useState<DatasetExport | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmId, setConfirmId] = useState<number | null>(null);
  const [role, setRole] = useState<"positive" | "negative">("positive");
  const [library, setLibrary] = useState<FrameView[]>([]);
  const [frameLimit, setFrameLimit] = useState(FRAME_PAGE);
  const [rename, setRename] = useState("");
  const [advice, setAdvice] = useState<ClassAdvice[]>([]);
  const [grabShortcut, setGrabShortcut] = useState<string | null>(readGrabShortcut);
  const [recordingShortcut, setRecordingShortcut] = useState(false);
  const [sources, setSources] = useState<SourceView[]>([]);
  const [sourceId, setSourceId] = useState<number | null>(null);
  const [settingsReady, setSettingsReady] = useState(!isDesktop);
  const [sourcesReady, setSourcesReady] = useState(!isDesktop);
  const [savedSourceId, setSavedSourceId] = useState<number | null>(null);
  const [coverageClassId, setCoverageClassId] = useState<number | null>(null);
  const [coverageExamples, setCoverageExamples] = useState<ExampleView[]>([]);
  const [coverageBusy, setCoverageBusy] = useState(false);
  const [coverageMore, setCoverageMore] = useState(false);
  const [coverageLoading, setCoverageLoading] = useState(false);
  const [coverageDeleteId, setCoverageDeleteId] = useState<number | null>(null);
  const [cleanupConfirm, setCleanupConfirm] = useState(false);
  const [cleanupNote, setCleanupNote] = useState<string | null>(null);
  const [mergeTarget, setMergeTarget] = useState("");
  const [mergeConfirm, setMergeConfirm] = useState(false);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const pendingRef = useRef(pending);
  const busyRef = useRef(busy);

  useEffect(() => {
    pendingRef.current = pending;
    busyRef.current = busy;
  }, [busy, pending]);

  useEffect(() => {
    if (!isDesktop) {
      setSettingsReady(true);
      return;
    }
    let alive = true;
    grabSettings()
      .then((settings) => {
        if (!alive) return;
        setGrabShortcut(settings.shortcut ? normalizeGrabShortcut(settings.shortcut) : null);
        setSavedSourceId(settings.sourceId);
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      })
      .finally(() => {
        if (alive) setSettingsReady(true);
      });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    window.localStorage.setItem(GRAB_SHORTCUT_KEY, grabShortcut ?? "none");
    if (!isDesktop || !settingsReady) return;
    let alive = true;
    saveGrabShortcut(grabShortcut).catch((reason: unknown) => {
      if (alive) setError(String(reason));
    });
    return () => {
      alive = false;
    };
  }, [grabShortcut, settingsReady]);

  // The global shortcut grabs from the source chosen here, so the picker and
  // the destination are stored together.
  useEffect(() => {
    if (projectId == null) {
      setSources([]);
      setSourceId(null);
      setSourcesReady(true);
      return;
    }
    setSourcesReady(false);
    let alive = true;
    listSources(projectId)
      .then((rows) => {
        if (!alive) return;
        setSources(rows);
        setSourceId((current) => {
          if (current !== null && rows.some((row) => row.id === current)) return current;
          if (savedSourceId !== null && rows.some((row) => row.id === savedSourceId)) {
            return savedSourceId;
          }
          return null;
        });
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      })
      .finally(() => {
        if (alive) setSourcesReady(true);
      });
    return () => {
      alive = false;
    };
  }, [projectId, savedSourceId]);

  useEffect(() => {
    if (!isDesktop || !settingsReady || !sourcesReady || projectId == null) return;
    let alive = true;
    setGrabTarget(projectId, sourceId).catch((reason: unknown) => {
      if (alive) setError(String(reason));
    });
    return () => {
      alive = false;
    };
  }, [projectId, settingsReady, sourceId, sourcesReady]);

  // Coverage and threshold advice follow the class list, so every save or
  // threshold change refreshes it without a separate button.
  useEffect(() => {
    if (!projectId) {
      setAdvice([]);
      return;
    }
    let alive = true;
    classAdvice(projectId)
      .then((rows) => {
        if (alive) setAdvice(rows);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [projectId, classes]);

  async function applyAdvice(entry: ClassAdvice) {
    if (!projectId || entry.suggested === null) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await setClassThreshold(entry.classId, entry.suggested);
      setClasses((current) =>
        current.map((item) => (item.id === updated.id ? updated : item))
      );
      setSelected((current) => (current?.id === updated.id ? updated : current));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    if (!projectId) {
      setClasses([]);
      return;
    }
    let alive = true;
    listClasses(projectId)
      .then((rows) => {
        if (alive) setClasses(rows);
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      });
    // Examples saved before the matcher existed carry no fingerprint; filling
    // them in here keeps the first box already comparable.
    embedExamples(projectId).catch(() => {});
    listFrames(projectId, frameLimit)
      .then((rows) => {
        if (alive) setLibrary(rows);
      })
      .catch(() => {
        if (alive) setLibrary([]);
      });
    setFrame(null);
    setBox(null);
    setPending([]);
    setFullscreen(false);
    setSelected(null);
    setExamples([]);
    setMatches([]);
    setExportInfo(null);
    setClassName("");
    setRole("positive");
    setRename("");
    return () => {
      alive = false;
    };
  }, [projectId]);

  useEffect(() => {
    if (!selected) {
      setExamples([]);
      return;
    }
    let alive = true;
    listExamples(selected.id, EXAMPLES_PAGE)
      .then((rows) => {
        if (alive) {
          setExamples(rows);
          setExamplesMore(rows.length >= EXAMPLES_PAGE);
        }
      })
      .catch((reason: unknown) => {
        if (alive) setError(String(reason));
      });
    return () => {
      alive = false;
    };
  }, [selected]);

  useEffect(() => {
    setRename(selected?.name ?? "");
    setMergeTarget("");
    setMergeConfirm(false);
  }, [selected]);

  // A confirmation belongs to the frame it was started on.
  useEffect(() => {
    setConfirmDiscard(false);
  }, [frame?.id]);

  useEffect(() => {
    if (!projectId || !frame || !box) {
      setMatches([]);
      return;
    }
    let alive = true;
    matchRegion(projectId, frame.id, box)
      .then((rows) => {
        if (alive) setMatches(rows);
      })
      .catch(() => {
        if (alive) setMatches([]);
      });
    return () => {
      alive = false;
    };
  }, [projectId, frame, box]);

  useEffect(() => {
    if (!fullscreen) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setFullscreen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fullscreen]);

  useEffect(() => {
    // On desktop the registered global shortcut handles the key even while a
    // label input has focus; a second page handler would grab twice.
    if (isDesktop || !grabShortcut || recordingShortcut) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.repeat) return;
      if (!shortcutMatches(event, grabShortcut)) return;
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest("input, textarea, select, [contenteditable='true']")
      ) {
        return;
      }
      if (fullscreen || busy || !projectId || pending.length > 0) return;
      event.preventDefault();
      void grab();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    busy,
    fullscreen,
    grabShortcut,
    pending.length,
    projectId,
    recordingShortcut,
    sourceId,
  ]);

  // The Rust side saves the frame even when Label is not mounted or iRecon is
  // behind another window. This keeps an open Label page in step with it.
  useEffect(() => {
    if (!isDesktop) return;
    let alive = true;
    const cleanups: Array<() => void> = [];
    void onFrameGrabbed((next) => {
      if (!alive) return;
      setLibrary((current) =>
        current.some((entry) => entry.id === next.id) ? current : [next, ...current]
      );
      if (pendingRef.current.length === 0 && !busyRef.current) {
        setFrame(next);
        setBox(null);
        setPending([]);
      }
    }).then((off) => {
      if (alive) cleanups.push(off);
      else off();
    });
    void onGrabError((message) => {
      if (alive) setError(message);
    }).then((off) => {
      if (alive) cleanups.push(off);
      else off();
    });
    return () => {
      alive = false;
      for (const cleanup of cleanups) cleanup();
    };
  }, []);

  useEffect(() => {
    if (!recordingShortcut) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.repeat) return;
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape" || event.key === "Tab") {
        setRecordingShortcut(false);
        return;
      }
      if (event.key === "Backspace" || event.key === "Delete") {
        setGrabShortcut(null);
        setRecordingShortcut(false);
        return;
      }
      if (
        event.key === "Shift" ||
        event.key === "Control" ||
        event.key === "Alt" ||
        event.key === "Meta" ||
        event.key === "CapsLock" ||
        event.key === "Dead" ||
        event.key === "Unidentified"
      ) {
        return;
      }
      const key = shortcutLabel(event.key);
      const modifiers: string[] = [];
      if (event.ctrlKey) modifiers.push("Ctrl");
      if (event.altKey) modifiers.push("Alt");
      if (event.shiftKey) modifiers.push("Shift");
      if (event.metaKey) modifiers.push("Meta");
      if (modifiers.length === 0 && !isFunctionShortcut(key)) {
        modifiers.push("Ctrl", "Alt");
      }
      setGrabShortcut([...modifiers, key].join("+"));
      setRecordingShortcut(false);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recordingShortcut]);

  async function refreshClasses(id: number) {
    const rows = await listClasses(id);
    setClasses(rows);
    return rows;
  }

  async function resetDemo() {
    setBusy(true);
    setError(null);
    try {
      await resetDemoData();
      if (projectId) await refreshClasses(projectId);
      setSelected(null);
      setExamples([]);
      setMatches([]);
      setBox(null);
      setPending([]);
      setExportInfo(null);
      setClassName("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function grab() {
    if (!projectId || pending.length > 0) return;
    setBusy(true);
    setError(null);
    try {
      const next = await grabFrame(projectId, sourceId);
      setFrame(next);
      setBox(null);
      setPending([]);
      setLibrary((current) => [next, ...current]);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  // "Do not label this image": the screenshot is dropped and the next
  // unlabeled one opens, so a bad grab does not sit in the queue.
  // Older frames are fetched only when asked for, so a large project opens
  // without decoding thousands of previews first.
  async function loadOlder() {
    if (!projectId) return;
    const next = frameLimit + FRAME_PAGE;
    try {
      const rows = await listFrames(projectId, next);
      setLibrary(rows);
      setFrameLimit(next);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function discardFrame() {
    if (!projectId || !frame) return;
    setBusy(true);
    setError(null);
    try {
      await deleteFrame(frame.id);
      const rows = await listFrames(projectId, frameLimit);
      setLibrary(rows);
      const next = rows.find((entry) => entry.examples === 0) ?? null;
      if (next) {
        setFrame(await openFrame(next.id));
      } else {
        setFrame(null);
      }
      setBox(null);
      setPending([]);
      setConfirmDiscard(false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function open(id: number) {
    setBusy(true);
    setError(null);
    try {
      const next = await openFrame(id);
      setFrame(next);
      setBox(null);
      setPending([]);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function save() {
    if (!projectId || !frame) return;
    const queued = [...pending];
    let current: PendingBox | null = null;
    if (box && className.trim()) {
      const name = className.trim();
      pendingBoxId += 1;
      current = {
        id: `box-${pendingBoxId}`,
        box,
        className: name,
        role,
        color:
          classes.find((item) => item.name.toLowerCase() === name.toLowerCase())?.color ??
          "var(--primary)",
      };
      queued.push(current);
    }
    if (queued.length === 0) return;
    setBusy(true);
    setError(null);
    const saved = new Set<string>();
    let lastClass: LabelClass | null = null;
    let accounted = false;
    try {
      const byName = new Map<string, LabelClass>();
      for (const entry of queued) {
        const key = entry.className.toLowerCase();
        let target =
          byName.get(key) ?? classes.find((item) => item.name.toLowerCase() === key);
        if (!target) {
          target = await createClass(projectId, entry.className);
        }
        byName.set(key, target);
        await createExample(target.id, frame.id, entry.box, entry.role);
        saved.add(entry.id);
        lastClass = target;
      }
      setPending((rows) => rows.filter((entry) => !saved.has(entry.id)));
      if (current && saved.has(current.id)) setBox(null);
      setLibrary((rows) =>
        rows.map((entry) =>
          entry.id === frame.id ? { ...entry, examples: entry.examples + saved.size } : entry
        )
      );
      accounted = true;
      if (lastClass) {
        setSelected(lastClass);
        const rows = await listExamples(lastClass.id, EXAMPLES_PAGE).catch(() => null);
        if (rows) {
          setExamples(rows);
          setExamplesMore(rows.length >= EXAMPLES_PAGE);
        }
      }
      await refreshClasses(projectId);
    } catch (reason) {
      setPending((rows) => rows.filter((entry) => !saved.has(entry.id)));
      if (current && saved.has(current.id)) setBox(null);
      if (saved.size > 0 && !accounted) {
        setLibrary((rows) =>
          rows.map((entry) =>
            entry.id === frame.id ? { ...entry, examples: entry.examples + saved.size } : entry
          )
        );
        await refreshClasses(projectId).catch(() => {});
      }
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  function addBox() {
    if (!box || !className.trim()) return;
    const name = className.trim();
    pendingBoxId += 1;
    setPending((current) => [
      ...current,
      {
        id: `box-${pendingBoxId}`,
        box,
        className: name,
        role,
        color:
          classes.find((item) => item.name.toLowerCase() === name.toLowerCase())?.color ??
          "var(--primary)",
      },
    ]);
    setBox(null);
  }

  function removePending(id: string) {
    setPending((current) => current.filter((entry) => entry.id !== id));
  }

  async function exportData() {
    if (!projectId) return;
    setBusy(true);
    setError(null);
    try {
      const result = await exportDataset(projectId);
      setExportInfo(result);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function adjustThreshold(item: LabelClass, delta: number) {
    const next = Math.min(1, Math.max(0, Math.round((item.threshold + delta) * 100) / 100));
    if (next === item.threshold) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await setClassThreshold(item.id, next);
      setClasses((current) =>
        current.map((entry) => (entry.id === updated.id ? updated : entry))
      );
      setSelected((current) => (current?.id === updated.id ? updated : current));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function removeClass(id: number) {
    setBusy(true);
    try {
      await deleteClass(id);
      if (projectId) await refreshClasses(projectId);
      if (selected?.id === id) setSelected(null);
      if (coverageClassId === id) {
        setCoverageClassId(null);
        setCoverageExamples([]);
        setCoverageMore(false);
      }
      setConfirmId(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function toggleCoverage(classId: number) {
    if (coverageClassId === classId) {
      setCoverageClassId(null);
      setCoverageExamples([]);
      setCoverageMore(false);
      setCoverageDeleteId(null);
      return;
    }
    setCoverageClassId(classId);
    setCoverageBusy(true);
    setCoverageMore(false);
    setCoverageDeleteId(null);
    setError(null);
    try {
      const rows = await listExamples(classId, EXAMPLES_PAGE);
      setCoverageExamples(rows);
      setCoverageMore(rows.length >= EXAMPLES_PAGE);
    } catch (reason) {
      setError(String(reason));
      setCoverageExamples([]);
    } finally {
      setCoverageBusy(false);
    }
  }

  async function loadOlderCoverage() {
    if (coverageClassId === null || coverageLoading || !coverageMore) return;
    const oldest = coverageExamples[coverageExamples.length - 1];
    if (!oldest) return;
    setCoverageLoading(true);
    try {
      const rows = await listExamples(coverageClassId, EXAMPLES_PAGE, oldest.id);
      setCoverageExamples((current) => {
        const seen = new Set(current.map((entry) => entry.id));
        return [...current, ...rows.filter((entry) => !seen.has(entry.id))];
      });
      setCoverageMore(rows.length >= EXAMPLES_PAGE);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setCoverageLoading(false);
    }
  }

  async function removeCoverageExample(example: ExampleView) {
    if (coverageDeleteId !== example.id) {
      setCoverageDeleteId(example.id);
      return;
    }
    setCoverageBusy(true);
    setError(null);
    try {
      await deleteExample(example.id);
      setCoverageExamples((current) => current.filter((entry) => entry.id !== example.id));
      setCoverageDeleteId(null);
      if (selected?.id === example.classId) {
        const rows = await listExamples(example.classId, EXAMPLES_PAGE).catch(() => null);
        if (rows) {
          setExamples(rows);
          setExamplesMore(rows.length >= EXAMPLES_PAGE);
        }
      }
      if (projectId) {
        await refreshClasses(projectId);
        const rows = await listFrames(projectId, frameLimit).catch(() => null);
        if (rows) setLibrary(rows);
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setCoverageBusy(false);
    }
  }

  async function cleanClasses() {
    if (!projectId) return;
    if (!cleanupConfirm) {
      setCleanupConfirm(true);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const report = await cleanupClasses(projectId);
      const parts: string[] = [];
      if (report.removedClasses > 0) parts.push(`${report.removedClasses} removed`);
      if (report.mergedClasses > 0) parts.push(`${report.mergedClasses} merged`);
      if (report.movedExamples > 0) parts.push(`${report.movedExamples} examples moved`);
      setCleanupNote(parts.length > 0 ? `${parts.join(", ")}.` : "Nothing to clean up.");
      setCleanupConfirm(false);
      setSelected(null);
      setCoverageClassId(null);
      setCoverageExamples([]);
      setCoverageMore(false);
      await refreshClasses(projectId);
    } catch (reason) {
      setError(String(reason));
      setCleanupConfirm(false);
    } finally {
      setBusy(false);
    }
  }

  async function mergeSelected(item: LabelClass) {
    const targetId = Number(mergeTarget);
    if (!targetId || targetId === item.id) return;
    if (!mergeConfirm) {
      setMergeConfirm(true);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await mergeClasses(item.id, targetId);
      setMergeConfirm(false);
      setMergeTarget("");
      setSelected(null);
      setCoverageClassId(null);
      setCoverageExamples([]);
      setCoverageMore(false);
      if (projectId) await refreshClasses(projectId);
    } catch (reason) {
      setError(String(reason));
      setMergeConfirm(false);
    } finally {
      setBusy(false);
    }
  }

  async function renameSelected(item: LabelClass) {
    const name = rename.trim();
    if (!name || name === item.name) return;
    setBusy(true);
    setError(null);
    try {
      const updated = await updateClass(item.id, { name });
      setClasses((current) =>
        current.map((entry) => (entry.id === updated.id ? updated : entry))
      );
      setSelected(updated);
      setClassName(updated.name);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function recolour(item: LabelClass, color: string) {
    setBusy(true);
    setError(null);
    try {
      const updated = await updateClass(item.id, { color });
      setClasses((current) =>
        current.map((entry) => (entry.id === updated.id ? updated : entry))
      );
      setSelected(updated);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const positives = examples.filter((example) => example.role !== "negative");
  const negatives = examples.filter((example) => example.role === "negative");
  const unlabeled = library.filter((entry) => entry.examples === 0).length;
  const nextUnlabeled =
    library.find((entry) => entry.examples === 0 && entry.id !== frame?.id) ?? null;
  // `frame.examples` is a snapshot from when the frame opened, so read the
  // count the library keeps after each save.
  const frameLabels = frame
    ? library.find((entry) => entry.id === frame.id)?.examples ?? frame.examples
    : 0;
  const readyCount = pending.length + (box && className.trim() ? 1 : 0);
  async function loadOlderExamples() {
    if (!selected || examplesLoading) return;
    const beforeId = examples[examples.length - 1]?.id;
    if (!beforeId) return;
    setExamplesLoading(true);
    try {
      const rows = await listExamples(selected.id, EXAMPLES_PAGE, beforeId);
      setExamples((current) => [
        ...current,
        ...rows.filter((row) => !current.some((entry) => entry.id === row.id)),
      ]);
      setExamplesMore(rows.length >= EXAMPLES_PAGE);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setExamplesLoading(false);
    }
  }

  const pendingOverlay = pending.map((entry) => ({ box: entry.box, color: entry.color }));
  const projectName = projects.find((project) => project.id === projectId)?.name;
  const fullscreenButton = (
    <Button
      size="sm"
      variant="outline"
      className="gap-1.5"
      disabled={!frame}
      onClick={() => setFullscreen(true)}
    >
      <Maximize2 className="size-4" aria-hidden="true" />
      Full screen
    </Button>
  );

  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Label"
        description="Grab a frame, draw a box, name it. Each save becomes an example for the matcher."
        action={
          <>
            <Button
              size="sm"
              variant="outline"
              className="gap-1.5"
              disabled={!projectId || busy}
              onClick={exportData}
            >
              <Download className="size-4" aria-hidden="true" />
              Export dataset
            </Button>
            <Button
              size="sm"
              className="gap-1.5"
              disabled={!projectId || busy || pending.length > 0}
              aria-keyshortcuts={grabShortcut ?? undefined}
              title={
                pending.length > 0
                  ? "Save or remove the boxes on this frame first"
                  : grabShortcut
                    ? `Grab frame (${grabShortcut})`
                    : "Grab frame"
              }
              onClick={grab}
            >
              <Crop className="size-4" aria-hidden="true" />
              {busy ? "Working" : "Grab frame"}
              {grabShortcut ? (
                <kbd className="rounded border border-current/30 px-1.5 text-xs font-medium opacity-70">
                  {grabShortcut}
                </kbd>
              ) : null}
            </Button>
          </>
        }
      />

      {exportInfo ? (
        <p className="border-b border-border p-2 text-xs text-muted-foreground tabular-nums">
          Exported {exportInfo.frames} frames, {exportInfo.boxes} boxes, {exportInfo.skipped}{" "}
          skipped.
        </p>
      ) : null}

      {error ? (
        <p role="alert" className="border-b border-border p-2 text-sm text-destructive">
          {error}
        </p>
      ) : null}

      <div className="flex flex-col gap-2.5 p-2.5">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs text-muted-foreground">Project</span>
          {projects.length === 0 ? (
            <span className="text-sm text-muted-foreground">
              Create a project on the Projects page first.
            </span>
          ) : (
            projects.map((project) => (
              <Button
                key={project.id}
                size="sm"
                variant={project.id === projectId ? "secondary" : "ghost"}
                disabled={live}
                onClick={() => selectProject(project.id)}
              >
                {project.name}
              </Button>
            ))
          )}
          {!isDesktop ? (
            <Button
              size="sm"
              variant="ghost"
              className="ml-auto gap-1.5"
              disabled={busy}
              onClick={resetDemo}
            >
              <RotateCcw className="size-4" aria-hidden="true" />
              Reset demo
            </Button>
          ) : null}
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs text-muted-foreground">Source</span>
          <Button
            size="sm"
            variant={sourceId === null ? "secondary" : "ghost"}
            title="Grab the primary display"
            onClick={() => setSourceId(null)}
          >
            Primary display
          </Button>
          {sources.map((source) => (
            <Button
              key={source.id}
              size="sm"
              variant={source.id === sourceId ? "secondary" : "ghost"}
              title={`${source.kind}: ${source.target}`}
              onClick={() => setSourceId(source.id)}
            >
              {source.label}
            </Button>
          ))}
          {sources.length === 0 ? (
            <span className="text-xs text-muted-foreground">
              Add a display, window, file or folder source on Sources to grab from it.
            </span>
          ) : null}
          <span className="ml-auto text-xs text-muted-foreground">
            The global shortcut uses this project and source.
          </span>
        </div>

        <div className="grid gap-2.5 lg:grid-cols-[1.6fr_1fr]">
          <Card className="min-w-0">
            <CardHeader className="flex flex-row items-start justify-between gap-2">
              <div className="flex flex-col gap-1.5">
                <CardTitle>Frame</CardTitle>
                <CardDescription>
                  Draw a box, add it, then draw the next object. The crop comes from the
                  full resolution frame, not the preview.
                </CardDescription>
              </div>
              {frame ? (
                <div className="flex flex-wrap items-center gap-1.5">
                  <Badge variant="outline" className="tabular-nums">
                    {frame.width} by {frame.height}
                  </Badge>
                  <Button
                    size="sm"
                    variant={confirmDiscard ? "destructive" : "ghost"}
                    className="gap-1.5"
                    disabled={busy || pending.length > 0 || frameLabels > 0}
                    title={
                      frameLabels > 0
                        ? "This screenshot already has labels. Labeled screenshots are removed from Library."
                        : pending.length > 0
                          ? "Save or remove the boxes on this frame first"
                          : "Delete this screenshot without labeling it"
                    }
                    onClick={() => {
                      if (confirmDiscard) void discardFrame();
                      else setConfirmDiscard(true);
                    }}
                  >
                    <Trash2 className="size-4" aria-hidden="true" />
                    {confirmDiscard ? "Delete" : "Delete screenshot"}
                  </Button>
                  {confirmDiscard ? (
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      onClick={() => setConfirmDiscard(false)}
                    >
                      Cancel
                    </Button>
                  ) : null}
                </div>
              ) : null}
            </CardHeader>
            <CardContent className="min-w-0">
              {frame ? (
                <FrameStage
                  frame={frame}
                  box={box}
                  onBox={setBox}
                  overlay={pendingOverlay}
                  action={fullscreenButton}
                />
              ) : (
                <Skeleton className="aspect-video w-full" />
              )}
              {library.length > 0 ? (
                <div className="flex flex-col gap-1.5">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-xs text-muted-foreground">Captured</span>
                    <Badge variant="outline" className="tabular-nums">
                      {unlabeled} unlabeled
                    </Badge>
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={busy || pending.length > 0 || !nextUnlabeled}
                      title={
                        pending.length > 0
                          ? "Save or remove the boxes on this frame first"
                          : "Open the next unlabeled frame"
                      }
                      onClick={() => {
                        if (nextUnlabeled) void open(nextUnlabeled.id);
                      }}
                    >
                      Next unlabeled
                    </Button>
                  </div>
                  {library.length >= frameLimit ? (
                    <Button
                      size="sm"
                      variant="outline"
                      className="self-start"
                      disabled={busy}
                      onClick={loadOlder}
                    >
                      Load older
                    </Button>
                  ) : null}
                  <div className="flex gap-1.5 overflow-x-auto pb-1.5">
                    {library.map((entry) => (
                      <Button
                        key={entry.id}
                        size="icon"
                        variant={frame?.id === entry.id ? "secondary" : "outline"}
                        className="size-9 shrink-0"
                        aria-label={`Open frame ${entry.id}, ${entry.examples} labels`}
                        title={`Frame ${entry.id}, ${entry.examples} label${entry.examples === 1 ? "" : "s"}`}
                        disabled={busy || pending.length > 0}
                        onClick={() => open(entry.id)}
                      >
                        <img
                          src={frameImage(entry)}
                          alt=""
                          loading="lazy"
                          decoding="async"
                          className="size-8 rounded-sm object-cover"
                        />
                      </Button>
                    ))}
                  </div>
                </div>
              ) : null}
              <div className="flex flex-col gap-2">
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <p className="max-w-[52ch] text-xs text-muted-foreground">
                    {frame
                      ? box
                        ? `Box ${(box.w * 100).toFixed(1)}% wide, ${(box.h * 100).toFixed(1)}% tall. Add it to the frame, then draw the next object.`
                        : pending.length > 0
                          ? `${pending.length} box${pending.length === 1 ? "" : "es"} on this frame. Draw another, or save them together.`
                          : "No box yet. Shift-drag to pan, scroll to zoom, then drag over the object."
                      : grabShortcut
                        ? `Press Grab frame or ${grabShortcut}. It also works while iRecon is behind another window.`
                        : "Press Grab frame. It also works while iRecon is behind another window."}
                  </p>
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="text-xs text-muted-foreground">Grab shortcut</span>
                    <Button
                      size="sm"
                      variant="outline"
                      className="min-w-24 gap-1.5"
                      aria-label="Change grab frame shortcut"
                      aria-pressed={recordingShortcut}
                      title="Click, then press a key. Ctrl or Alt is added to a bare key so the global grab does not block typing. Backspace sets None. Escape cancels."
                      onClick={() => setRecordingShortcut((current) => !current)}
                      onBlur={() => setRecordingShortcut(false)}
                    >
                      <Keyboard className="size-4" aria-hidden="true" />
                      {recordingShortcut ? "Press a key" : grabShortcut ?? "None"}
                    </Button>
                    {grabShortcut ? (
                      <Button
                        size="sm"
                        variant="ghost"
                        title="Disable the grab frame shortcut"
                        onClick={() => setGrabShortcut(null)}
                      >
                        None
                      </Button>
                    ) : null}
                  </div>
                </div>
                {matches.length > 0 ? (
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-xs text-muted-foreground">Nearest</span>
                    {matches.slice(0, 3).map((match) => (
                      <Badge
                        key={match.classId}
                        variant={match.score >= 0.85 ? "secondary" : "outline"}
                        className="tabular-nums"
                      >
                        {match.name} {match.score.toFixed(2)}
                      </Badge>
                    ))}
                  </div>
                ) : null}
              </div>
            </CardContent>
          </Card>

          <Card className="min-w-0">
            <CardHeader>
              <CardTitle>Label boxes</CardTitle>
              <CardDescription>
                Pick or create a class, add each box, then save them together. A new name
                creates the class on save.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <div className="flex flex-col gap-1.5">
                <label htmlFor="class-name" className="text-xs text-muted-foreground">
                  Class name
                </label>
                <Input
                  id="class-name"
                  value={className}
                  onChange={(event) => setClassName(event.target.value)}
                  placeholder="red car"
                  maxLength={40}
                />
              </div>
              {classes.length > 0 ? (
                <div className="flex flex-wrap items-center gap-2">
                  {classes.map((item) => (
                    <div key={item.id} className="flex items-center gap-1.5">
                      <Button
                        size="sm"
                        variant={
                          className.toLowerCase() === item.name.toLowerCase()
                            ? "secondary"
                            : "outline"
                        }
                        className="gap-1.5"
                        onClick={() => {
                          setClassName(item.name);
                          setSelected(item);
                          setRename(item.name);
                        }}
                      >
                        <span
                          className="size-2 rounded-full"
                          style={{ backgroundColor: item.color }}
                          aria-hidden="true"
                        />
                        {item.name}
                        <Badge variant="secondary" className="tabular-nums">
                          {item.examples}
                        </Badge>
                        <Badge variant="outline" className="tabular-nums">
                          {Math.round(item.threshold * 100)}%
                        </Badge>
                      </Button>
                      <Button
                        size="icon"
                        variant="ghost"
                        aria-label={`Rename ${item.name}`}
                        onClick={() => {
                          setSelected(item);
                          setRename(item.name);
                          document.getElementById("class-rename")?.focus();
                        }}
                      >
                        <Pencil className="size-4" aria-hidden="true" />
                      </Button>
                      <Button
                        size="icon"
                        variant="ghost"
                        aria-label={`Delete ${item.name}`}
                        onClick={() => setConfirmId(confirmId === item.id ? null : item.id)}
                      >
                        <Trash2 className="size-4" aria-hidden="true" />
                      </Button>
                      {confirmId === item.id ? (
                        <Button
                          size="sm"
                          variant="destructive"
                          disabled={busy}
                          onClick={() => removeClass(item.id)}
                        >
                          Delete
                        </Button>
                      ) : null}
                    </div>
                  ))}
                  <div className="flex w-full flex-wrap items-center gap-2">
                    <Button
                      size="sm"
                      variant={cleanupConfirm ? "destructive" : "ghost"}
                      disabled={busy}
                      onClick={() => void cleanClasses()}
                    >
                      {cleanupConfirm ? "Confirm cleanup" : "Clean up classes"}
                    </Button>
                    {cleanupNote ? (
                      <span className="text-xs text-muted-foreground">{cleanupNote}</span>
                    ) : null}
                  </div>
                </div>
              ) : (
                <p className="text-xs text-muted-foreground">
                  No classes yet. The first save creates one.
                </p>
              )}
              {pending.length > 0 ? (
                <div className="flex flex-col gap-1.5">
                  <span className="text-xs text-muted-foreground">On this frame</span>
                  <ul className="overflow-hidden rounded-md border border-border">
                    {pending.map((entry, index) => (
                      <li key={entry.id}>
                        {index > 0 ? <Separator /> : null}
                        <div className="flex flex-wrap items-center gap-2 p-2">
                          <span
                            className="size-2 rounded-full"
                            style={{ backgroundColor: entry.color }}
                            aria-hidden="true"
                          />
                          <span className="min-w-0 flex-1 truncate text-sm">
                            {entry.className}
                          </span>
                          <Badge variant={entry.role === "positive" ? "secondary" : "outline"}>
                            {entry.role}
                          </Badge>
                          <span className="text-xs text-muted-foreground tabular-nums">
                            {Math.round(entry.box.w * 100)}% by {Math.round(entry.box.h * 100)}%
                          </span>
                          <Button
                            size="icon"
                            variant="ghost"
                            aria-label={`Remove ${entry.className} box`}
                            disabled={busy}
                            onClick={() => removePending(entry.id)}
                          >
                            <Trash2 className="size-4" aria-hidden="true" />
                          </Button>
                        </div>
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  size="sm"
                  variant={role === "positive" ? "secondary" : "outline"}
                  onClick={() => setRole("positive")}
                >
                  Positive
                </Button>
                <Button
                  size="sm"
                  variant={role === "negative" ? "secondary" : "outline"}
                  onClick={() => setRole("negative")}
                >
                  Negative
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  className="gap-1.5"
                  disabled={!frame || !box || !className.trim() || busy}
                  onClick={addBox}
                >
                  <Plus className="size-4" aria-hidden="true" />
                  Add box
                </Button>
                <Button
                  size="sm"
                  disabled={!frame || readyCount === 0 || busy}
                  onClick={save}
                >
                  {readyCount === 1
                    ? "Save example"
                    : readyCount === 0
                      ? "Save examples"
                      : `Save ${readyCount} examples`}
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                {pending.length > 0
                  ? `${pending.length} box${pending.length === 1 ? "" : "es"} on this frame. Save them together, or draw another.`
                  : role === "positive"
                    ? "A positive is the thing itself, which is what teaches the matcher to box it."
                    : "A negative looks close but is not it, so saving one raises the bar for a match."}
              </p>
            </CardContent>
          </Card>
        </div>

        <Card>
          <CardHeader>
            <CardTitle>Coverage</CardTitle>
            <CardDescription>
              How your positives and negatives compare. The suggestion moves the
              threshold only when you apply it.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {advice.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                No classes to read yet.
              </p>
            ) : (
              <ul className="flex flex-col gap-2">
                {advice.map((entry) => (
                  <li key={entry.classId} className="rounded-lg border border-border p-2">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-sm font-medium">{entry.name}</span>
                      <Badge variant="secondary" className="tabular-nums">
                        {entry.positives} positive{entry.positives === 1 ? "" : "s"}
                      </Badge>
                      <Badge variant="outline" className="tabular-nums">
                        {entry.negatives} negative{entry.negatives === 1 ? "" : "s"}
                      </Badge>
                      {entry.positives >= 2 ? (
                        <Badge variant="outline" className="tabular-nums">
                          look-alike {Math.round(entry.similarity * 100)}%
                        </Badge>
                      ) : null}
                      <div className="ml-auto flex flex-wrap items-center gap-1.5">
                        <Button
                          size="sm"
                          variant="ghost"
                          aria-expanded={coverageClassId === entry.classId}
                          disabled={coverageBusy && coverageClassId !== entry.classId}
                          onClick={() => void toggleCoverage(entry.classId)}
                        >
                          {coverageClassId === entry.classId ? "Hide examples" : "View examples"}
                        </Button>
                        {entry.suggested !== null ? (
                          <Button
                            size="sm"
                            variant="outline"
                            className="tabular-nums"
                            disabled={busy || entry.suggested === entry.threshold}
                            onClick={() => applyAdvice(entry)}
                          >
                            Use {Math.round(entry.suggested * 100)}%
                          </Button>
                        ) : null}
                      </div>
                    </div>
                    {entry.hints.length > 0 ? (
                      <ul className="mt-1.5 flex list-disc flex-col gap-1.5 pl-2.5 text-xs text-muted-foreground">
                        {entry.hints.map((hint) => (
                          <li key={hint}>{hint}</li>
                        ))}
                      </ul>
                    ) : (
                      <p className="mt-1.5 text-xs text-muted-foreground">Looks healthy.</p>
                    )}
                    {coverageClassId === entry.classId ? (
                      <div className="mt-2 flex flex-col gap-2 border-t border-border pt-2">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-xs text-muted-foreground">
                            {coverageExamples.filter((example) => example.role !== "negative").length}{" "}
                            positives, {coverageExamples.filter((example) => example.role === "negative").length}{" "}
                            negatives. Delete anything that is mislabelled or too similar to another class.
                          </span>
                          <Button
                            size="sm"
                            variant="ghost"
                            className="ml-auto"
                            onClick={() => {
                              setCoverageClassId(null);
                              setCoverageExamples([]);
                              setCoverageDeleteId(null);
                            }}
                          >
                            Close
                          </Button>
                        </div>
                        {coverageBusy ? (
                          <Skeleton className="h-24 w-full" />
                        ) : coverageExamples.length === 0 ? (
                          <p className="text-sm text-muted-foreground">
                            No examples saved for this class yet.
                          </p>
                        ) : (
                          <ul className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-5">
                            {coverageExamples.map((example) => (
                              <li key={example.id} className="media-tile flex flex-col gap-1.5">
                                <img
                                  src={exampleImage(example)}
                                  alt={`${example.role} example from ${example.createdAt}`}
                                  loading="lazy"
                                  decoding="async"
                                  className="aspect-square w-full rounded-md border border-border object-cover"
                                />
                                <div className="flex flex-wrap items-center gap-1.5">
                                  <Badge
                                    variant={example.role === "negative" ? "outline" : "secondary"}
                                  >
                                    {example.role}
                                  </Badge>
                                  <span className="text-xs text-muted-foreground tabular-nums">
                                    {example.width} by {example.height}
                                  </span>
                                </div>
                                <Button
                                  size="sm"
                                  variant={coverageDeleteId === example.id ? "destructive" : "ghost"}
                                  className="gap-1.5"
                                  disabled={coverageBusy}
                                  aria-label={
                                    coverageDeleteId === example.id
                                      ? `Confirm delete example ${example.id}`
                                      : `Delete example ${example.id}`
                                  }
                                  onClick={() => void removeCoverageExample(example)}
                                >
                                  <Trash2 className="size-4" aria-hidden="true" />
                                  {coverageDeleteId === example.id ? "Confirm" : "Delete"}
                                </Button>
                              </li>
                            ))}
                          </ul>
                        )}
                        {coverageMore ? (
                          <div className="flex justify-end">
                            <Button
                              size="sm"
                              variant="outline"
                              disabled={coverageLoading}
                              onClick={() => void loadOlderCoverage()}
                            >
                              {coverageLoading ? "Loading..." : "Load older"}
                            </Button>
                          </div>
                        ) : null}
                      </div>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>

        {selected ? (
          <Card>
            <CardHeader className="flex flex-row items-start justify-between gap-2">
              <div className="flex flex-col gap-1.5">
                <CardTitle>{selected.name}</CardTitle>
                <CardDescription>
                  {positives.length} positives and {negatives.length} negatives. A box must
                  score at least {Math.round(selected.threshold * 100)}% against this class
                  to be drawn.
                </CardDescription>
              </div>
              <div className="flex items-center gap-1.5">
                <span className="text-xs text-muted-foreground">Threshold</span>
                <Button
                  size="icon"
                  variant="ghost"
                  aria-label={`Lower threshold for ${selected.name}`}
                  disabled={busy || selected.threshold <= 0}
                  onClick={() => adjustThreshold(selected, -0.05)}
                >
                  <Minus className="size-4" aria-hidden="true" />
                </Button>
                <Badge variant="outline" className="tabular-nums">
                  {Math.round(selected.threshold * 100)}%
                </Badge>
                <Button
                  size="icon"
                  variant="ghost"
                  aria-label={`Raise threshold for ${selected.name}`}
                  disabled={busy || selected.threshold >= 1}
                  onClick={() => adjustThreshold(selected, 0.05)}
                >
                  <Plus className="size-4" aria-hidden="true" />
                </Button>
              </div>
            </CardHeader>
            <CardContent className="flex flex-col gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <Input
                  id="class-rename"
                  value={rename}
                  onChange={(event) => setRename(event.target.value)}
                  aria-label={`Rename ${selected.name}`}
                  maxLength={40}
                  className="max-w-56"
                />
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy || !rename.trim() || rename.trim() === selected.name}
                  onClick={() => renameSelected(selected)}
                >
                  Rename
                </Button>
                <div className="flex flex-wrap items-center gap-1.5">
                  {CLASS_COLORS.map((color) => (
                    <button
                      key={color}
                      type="button"
                      aria-label={`Colour ${selected.name}`}
                      aria-pressed={selected.color === color}
                      className="size-6 rounded-full border border-border aria-pressed:ring-1 aria-pressed:ring-foreground"
                      style={{ backgroundColor: color }}
                      disabled={busy}
                      onClick={() => recolour(selected, color)}
                    />
                  ))}
                </div>
              </div>
              {classes.length > 1 ? (
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-xs text-muted-foreground">Merge into</span>
                  <select
                    aria-label={`Merge ${selected.name} into another class`}
                    value={mergeTarget}
                    onChange={(event) => {
                      setMergeTarget(event.target.value);
                      setMergeConfirm(false);
                    }}
                    className="h-8 rounded-md border border-border bg-background px-2 text-sm"
                  >
                    <option value="">Pick a class</option>
                    {classes
                      .filter((entry) => entry.id !== selected.id)
                      .map((entry) => (
                        <option key={entry.id} value={entry.id}>
                          {entry.name}
                        </option>
                      ))}
                  </select>
                  <Button
                    size="sm"
                    variant={mergeConfirm ? "destructive" : "outline"}
                    disabled={busy || !mergeTarget}
                    onClick={() => void mergeSelected(selected)}
                  >
                    {mergeConfirm ? "Confirm merge" : "Merge"}
                  </Button>
                </div>
              ) : null}
              <Separator />
              {examples.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  No examples saved for this class yet.
                </p>
              ) : (
                <div className="flex flex-col gap-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-xs text-muted-foreground">Positives</span>
                    <Badge variant="secondary" className="tabular-nums">
                      {positives.length}
                    </Badge>
                    <span className="text-xs text-muted-foreground">Negatives</span>
                    <Badge variant="outline" className="tabular-nums">
                      {negatives.length}
                    </Badge>
                  </div>
                  <ul className="grid grid-cols-3 gap-2 sm:grid-cols-4 lg:grid-cols-6">
                    {positives.map((example) => (
                      <ExampleTile key={example.id} example={example} />
                    ))}
                  </ul>
                  {negatives.length > 0 ? (
                    <>
                      <span className="text-xs text-muted-foreground">Negatives</span>
                      <ul className="grid grid-cols-3 gap-2 sm:grid-cols-4 lg:grid-cols-6">
                        {negatives.map((example) => (
                          <ExampleTile key={example.id} example={example} />
                        ))}
                      </ul>
                    </>
                  ) : null}
                  {examplesMore ? (
                    <Button
                      size="sm"
                      variant="outline"
                      className="self-start"
                      disabled={examplesLoading}
                      onClick={() => void loadOlderExamples()}
                    >
                      {examplesLoading ? "Loading" : "Load older"}
                    </Button>
                  ) : null}
                </div>
              )}
            </CardContent>
          </Card>
        ) : null}
      </div>

      {fullscreen && frame ? (
        <div className="fixed inset-0 z-50 flex flex-col gap-2 bg-background p-2.5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium">
                {projectName ? `${projectName} frame` : "Frame"}
              </span>
              <Badge variant="outline" className="tabular-nums">
                {frame.width} by {frame.height}
              </Badge>
            </div>
            <Button
              size="sm"
              variant="outline"
              className="gap-1.5"
              onClick={() => setFullscreen(false)}
            >
              <Minimize2 className="size-4" aria-hidden="true" />
              Close
            </Button>
          </div>
          <FrameStage
            frame={frame}
            box={box}
            onBox={setBox}
            overlay={pendingOverlay}
            fitToFrame={false}
            className="h-[calc(100dvh-8rem)]"
            action={
              <span className="text-xs text-muted-foreground">
                Escape closes. Shift-drag to pan, scroll to zoom.
              </span>
            }
          />
        </div>
      ) : null}
    </div>
  );
}
