import { listen } from "@tauri-apps/api/event";
import { useEffect, type RefObject } from "react";

type Point = { x: number; y: number };
export type ComposerDropEvent = {
  type: "enter" | "over" | "drop" | "leave";
  dropId?: string;
  position?: Point;
};

export function normalizeComposerDropEvent(raw: unknown): ComposerDropEvent | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  if (!["enter", "over", "drop", "leave"].includes(String(value.type))) return null;
  const type = value.type as ComposerDropEvent["type"];
  if (type === "leave") return { type };
  const position = value.position;
  if (!position || typeof position !== "object") return null;
  const point = position as Record<string, unknown>;
  if (typeof point.x !== "number" || typeof point.y !== "number") return null;
  const dropId = typeof value.dropId === "string" ? value.dropId : undefined;
  return { type, ...(dropId ? { dropId } : {}), position: { x: point.x, y: point.y } };
}

export function composerDropPointIsInside(
  bounds: Pick<DOMRect, "bottom" | "left" | "right" | "top">,
  point: Point | undefined,
) {
  if (!point) return false;
  return point.x >= bounds.left && point.x <= bounds.right &&
    point.y >= bounds.top && point.y <= bounds.bottom;
}

function pointIsInside(target: HTMLElement | null, point: Point | undefined) {
  return Boolean(
    target && composerDropPointIsInside(target.getBoundingClientRect(), point),
  );
}

function releaseDropListener(unlisten: (() => void) | null) {
  if (!unlisten) return;
  try {
    void Promise.resolve(unlisten()).catch(() => undefined);
  } catch {
    // A stale native listener must never break composer cleanup.
  }
}

export function useNativeComposerDrop(options: {
  disabled: boolean;
  onDrop: (dropId: string) => void | Promise<void>;
  setActive: (active: boolean) => void;
  targetRef: RefObject<HTMLElement | null>;
}) {
  const { disabled, onDrop, setActive, targetRef } = options;
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<unknown>("oomu://local-context-drag", ({ payload }) => {
      if (disposed) return;
      const event = normalizeComposerDropEvent(payload);
      if (!event || event.type === "leave") {
        setActive(false);
        return;
      }
      const inside = pointIsInside(targetRef.current, event.position);
      setActive(inside && !disabled && event.type !== "drop");
      // The native receipt proves that the operating system delivered a real
      // file drop to this OOMU window. The hook exists only while the chat
      // composer is mounted, so claim that exact receipt instead of guessing
      // from display scaling or racing a global "latest drop" lookup.
      if (event.type === "drop" && inside && !disabled) void onDrop(event.dropId ?? "");
    }).then((release) => {
      if (disposed) releaseDropListener(release); else unlisten = release;
    }).catch(() => setActive(false));
    return () => {
      disposed = true;
      releaseDropListener(unlisten);
    };
  }, [disabled, onDrop, setActive, targetRef]);
}
