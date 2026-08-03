"use client";

import { useCallback, useEffect, useRef, useState } from "react";

const STORAGE_KEY = "oomu.chat.dismissedSplitRoutes";
const MAX_ROUTES = 64;

function readStoredRoutes() {
  if (typeof window === "undefined") return new Set<string>();
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw || raw.length > 64 * 1024) return new Set<string>();
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set<string>();
    return new Set(parsed
      .filter((value): value is string => typeof value === "string" && value.length <= 2_048)
      .slice(-MAX_ROUTES));
  } catch {
    return new Set<string>();
  }
}

export function usePersistedDismissedSplitRoutes() {
  const [dismissedRoutes, setDismissedRoutes] = useState<Set<string>>(() => new Set());
  const [hasLoadedStoredRoutes, setHasLoadedStoredRoutes] = useState(false);
  const hasLoadedStoredRoutesRef = useRef(false);
  const pendingDismissalsRef = useRef(new Set<string>());
  const pendingRestoresRef = useRef(new Set<string>());

  useEffect(() => {
    let cancelled = false;
    queueMicrotask(() => {
      if (cancelled) return;
      const storedRoutes = readStoredRoutes();
      const pendingDismissals = [...pendingDismissalsRef.current];
      const pendingRestores = [...pendingRestoresRef.current];
      setDismissedRoutes((current) => {
        const next = new Set([...storedRoutes, ...current, ...pendingDismissals]);
        for (const identity of pendingRestores) next.delete(identity);
        return next;
      });
      hasLoadedStoredRoutesRef.current = true;
      pendingDismissalsRef.current.clear();
      pendingRestoresRef.current.clear();
      setHasLoadedStoredRoutes(true);
    });
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    if (!hasLoadedStoredRoutes || typeof window === "undefined") return;
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify([...dismissedRoutes].slice(-MAX_ROUTES)));
    } catch {
      // Preference persistence is best-effort when storage is unavailable.
    }
  }, [dismissedRoutes, hasLoadedStoredRoutes]);

  const dismissRoute = useCallback((identity: string) => {
    if (!hasLoadedStoredRoutesRef.current) {
      pendingRestoresRef.current.delete(identity);
      pendingDismissalsRef.current.add(identity);
    }
    setDismissedRoutes((current) => {
      if (current.has(identity)) return current;
      const next = new Set(current);
      next.add(identity);
      while (next.size > MAX_ROUTES) {
        const oldest = next.values().next().value;
        if (typeof oldest !== "string") break;
        next.delete(oldest);
      }
      return next;
    });
  }, []);

  const restoreRoute = useCallback((identity: string) => {
    if (!hasLoadedStoredRoutesRef.current) {
      pendingDismissalsRef.current.delete(identity);
      pendingRestoresRef.current.add(identity);
    }
    setDismissedRoutes((current) => {
      if (!current.has(identity)) return current;
      const next = new Set(current);
      next.delete(identity);
      return next;
    });
  }, []);

  return [dismissedRoutes, dismissRoute, restoreRoute] as const;
}
