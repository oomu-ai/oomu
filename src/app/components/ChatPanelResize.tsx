"use client";

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type RefObject,
} from "react";

const KEYBOARD_STEP = 16;

type Side = "left" | "right";

type UseResizablePanelOptions = {
  storageKey: string;
  defaultWidth: number;
  min: number;
  max: number;
  /**
   * Which edge the drag handle sits on. "right" = handle on the panel's trailing
   * edge (e.g. the sessions list) so dragging right widens it. "left" = handle on
   * the leading edge (e.g. the tuning drawer) so dragging left widens it.
   */
  side: Side;
  /**
   * The width currently rendered for this panel (after responsive fitting). Used as
   * the drag/keyboard starting point so the handle tracks what's on screen even when
   * a narrow window has shrunk the panel below its stored width.
   */
  liveWidthRef?: RefObject<number>;
};

type ResizablePanel = {
  width: number;
  isDragging: boolean;
  min: number;
  max: number;
  onPointerDown: (event: PointerEvent) => void;
  onKeyDown: (event: KeyboardEvent) => void;
};

function readStoredWidth(storageKey: string, fallback: number, min: number, max: number) {
  if (typeof window === "undefined") {
    return fallback;
  }
  try {
    const raw = window.localStorage.getItem(storageKey);
    const parsed = raw ? Number.parseInt(raw, 10) : Number.NaN;
    if (Number.isFinite(parsed)) {
      return Math.min(max, Math.max(min, parsed));
    }
  } catch {
    // Storage may be unavailable (private mode / sandbox) — fall back to default.
  }
  return fallback;
}

export function useResizablePanel(options: UseResizablePanelOptions): ResizablePanel {
  const { storageKey, defaultWidth, min, max, side, liveWidthRef } = options;
  const [width, setWidth] = useState(() => Math.min(max, Math.max(min, defaultWidth)));
  const [hasLoadedStoredWidth, setHasLoadedStoredWidth] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const dragState = useRef<{ startX: number; startWidth: number } | null>(null);

  const clamp = useCallback((value: number) => Math.min(max, Math.max(min, value)), [max, min]);

  const base = useCallback(() => {
    const live = liveWidthRef?.current;
    return live && live > 0 ? live : width;
  }, [liveWidthRef, width]);

  const onPointerDown = useCallback(
    (event: PointerEvent) => {
      if (event.button !== 0) {
        return;
      }
      event.preventDefault();
      dragState.current = { startX: event.clientX, startWidth: base() };
      setIsDragging(true);
    },
    [base],
  );

  const onKeyDown = useCallback(
    (event: KeyboardEvent) => {
      const widen = side === "right" ? "ArrowRight" : "ArrowLeft";
      const narrow = side === "right" ? "ArrowLeft" : "ArrowRight";
      if (event.key === widen) {
        event.preventDefault();
        setWidth(clamp(base() + KEYBOARD_STEP));
      } else if (event.key === narrow) {
        event.preventDefault();
        setWidth(clamp(base() - KEYBOARD_STEP));
      } else if (event.key === "Home") {
        event.preventDefault();
        setWidth(clamp(defaultWidth));
      }
    },
    [side, clamp, base, defaultWidth],
  );

  useEffect(() => {
    if (!isDragging) {
      return;
    }
    function handleMove(event: globalThis.PointerEvent) {
      const state = dragState.current;
      if (!state) {
        return;
      }
      const delta = side === "left" ? state.startX - event.clientX : event.clientX - state.startX;
      setWidth(clamp(state.startWidth + delta));
    }
    function stop() {
      dragState.current = null;
      setIsDragging(false);
    }
    const previousCursor = document.body.style.cursor;
    const previousSelect = document.body.style.userSelect;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    window.addEventListener("pointermove", handleMove);
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
    return () => {
      document.body.style.cursor = previousCursor;
      document.body.style.userSelect = previousSelect;
      window.removeEventListener("pointermove", handleMove);
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
    };
  }, [isDragging, side, clamp]);

  useEffect(() => {
    let cancelled = false;
    queueMicrotask(() => {
      if (cancelled) {
        return;
      }
      setWidth(readStoredWidth(storageKey, defaultWidth, min, max));
      setHasLoadedStoredWidth(true);
    });
    return () => {
      cancelled = true;
    };
  }, [storageKey, defaultWidth, min, max]);

  useEffect(() => {
    if (!hasLoadedStoredWidth) {
      return;
    }
    if (typeof window === "undefined") {
      return;
    }
    try {
      window.localStorage.setItem(storageKey, String(width));
    } catch {
      // Persisting the width is best-effort; ignore storage failures.
    }
  }, [hasLoadedStoredWidth, storageKey, width]);

  return { width, isDragging, min, max, onPointerDown, onKeyDown };
}

/** Tracks an element's content width via ResizeObserver. Returns 0 until measured. */
export function useContainerWidth(ref: RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const element = ref.current;
    if (!element || typeof ResizeObserver === "undefined") {
      return;
    }
    const update = () => setWidth(element.clientWidth);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}

type FitOptions = {
  mainMin: number;
  sessionsMin: number;
  tuningMin: number;
  splitMin?: number;
  splitOpen?: boolean;
  splitStored?: number;
};

/**
 * Resolves the on-screen widths of the side panels so the chat column always
 * keeps at least `mainMin` pixels. When the window is wide everyone gets their
 * stored width; when it narrows the panels give way (down to their own minimums)
 * before the chat does, so the tuning panel is never clipped.
 */
export function fitChatPanels(
  containerWidth: number,
  sessionsStored: number,
  tuningStored: number,
  drawerOpen: boolean,
  {
    mainMin,
    sessionsMin,
    tuningMin,
    splitMin: splitMinOption = 0,
    splitOpen = false,
    splitStored = splitMinOption,
  }: FitOptions,
): { sessions: number; tuning: number; split: number } {
  let sessions = sessionsStored;
  let tuning = drawerOpen ? tuningStored : 0;
  let split = splitOpen ? splitStored : 0;
  if (!containerWidth || containerWidth <= 0) {
    return { sessions, tuning, split };
  }
  const splitMin = splitOpen ? splitMinOption : 0;
  const handles = 1 + (drawerOpen ? 1 : 0) + (splitOpen ? 1 : 0);
  const tMin = drawerOpen ? tuningMin : 0;
  const available = containerWidth - mainMin - handles;
  if (sessions + split + tuning > available) {
    const target = Math.max(available, sessionsMin + splitMin + tMin);
    const need = sessions + split + tuning - target;
    const flexSessions = Math.max(0, sessions - sessionsMin);
    const flexSplit = Math.max(0, split - splitMin);
    const flexTuning = Math.max(0, tuning - tMin);
    const flexTotal = flexSessions + flexSplit + flexTuning;
    if (need > 0 && flexTotal > 0) {
      const shrink = Math.min(need, flexTotal);
      sessions = Math.round(sessions - (shrink * flexSessions) / flexTotal);
      split = Math.round(split - (shrink * flexSplit) / flexTotal);
      tuning = Math.round(tuning - (shrink * flexTuning) / flexTotal);
    }
  }
  return { sessions, tuning, split };
}

type ResizeHandleProps = {
  panel: ResizablePanel;
  label: string;
  /** The width actually rendered (for the aria value); defaults to the stored width. */
  value?: number;
};

export function ResizeHandle({ panel, label, value }: ResizeHandleProps) {
  return (
    <div
      aria-label={label}
      aria-orientation="vertical"
      aria-valuemax={panel.max}
      aria-valuemin={panel.min}
      aria-valuenow={Math.round(value ?? panel.width)}
      className={`group relative z-20 w-px shrink-0 cursor-col-resize touch-none self-stretch transition-colors hover:bg-[var(--accent)] focus:outline-none focus-visible:bg-[var(--accent)] ${
        panel.isDragging ? "bg-[var(--accent)]" : "bg-[var(--border-soft)]"
      }`}
      onKeyDown={panel.onKeyDown}
      onPointerDown={panel.onPointerDown}
      role="separator"
      tabIndex={0}
    >
      {/* Widened, invisible hit target so the 1px line is easy to grab. */}
      <span aria-hidden="true" className="absolute inset-y-0 -left-2 -right-2" />
    </div>
  );
}
