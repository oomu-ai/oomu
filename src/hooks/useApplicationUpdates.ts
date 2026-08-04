"use client";

import {
  useCallback,
  useEffect,
  useState,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from "react";
import { invoke } from "@/lib/invoke";
import {
  checkResultView,
  type ApplicationUpdateCheckResult,
  type ApplicationUpdateInstallEvent,
  type ApplicationUpdateView,
} from "@/lib/applicationUpdates";

const CHECK_EVENT = "oomu://check-for-updates";
const AUTOMATIC_RESULT_EVENT = "oomu://application-update-result";
const INSTALL_EVENT = "oomu://application-update-install";
const UI_READINESS_EVENT = "oomu://application-update-readiness";

type NavigationGuard = MutableRefObject<((proceed: () => void) => void) | null>;

function useNativeUpdateEvents(
  setView: Dispatch<SetStateAction<ApplicationUpdateView | null>>,
  setUiReady: Dispatch<SetStateAction<boolean>>,
  checkNow: () => Promise<void>,
) {
  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    async function register() {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const menu = await listen(CHECK_EVENT, () => void checkNow());
        const result = await listen<ApplicationUpdateCheckResult>(
          AUTOMATIC_RESULT_EVENT,
          ({ payload }) => setView(checkResultView(payload)),
        );
        const install = await listen<ApplicationUpdateInstallEvent>(
          INSTALL_EVENT,
          ({ payload }) => setView((current) => ({
            ...current,
            status: payload.status,
            currentVersion: current?.currentVersion ?? "",
            downloadedBytes: payload.downloadedBytes,
            totalBytes: payload.totalBytes,
            publicCode: payload.publicCode,
            retryable: payload.retryable,
          })),
        );
        const readiness = await listen<boolean>(
          UI_READINESS_EVENT,
          ({ payload }) => setUiReady(payload),
        );
        if (cancelled) [menu, result, install, readiness].forEach((unlisten) => unlisten());
        else unlisteners.push(menu, result, install, readiness);
      } catch {
        // Native update events exist only inside the Tauri desktop runtime.
      }
    }
    void register();
    return () => {
      cancelled = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [checkNow, setUiReady, setView]);
}

export function useApplicationUpdates(navGuardRef: NavigationGuard) {
  const [view, setView] = useState<ApplicationUpdateView | null>(null);
  const [uiReady, setUiReady] = useState(true);

  const checkNow = useCallback(async () => {
    setView((current) => ({
      status: "checking",
      currentVersion: current?.currentVersion ?? "",
    }));
    try {
      const result = await invoke<ApplicationUpdateCheckResult>("check_for_application_update");
      setView(checkResultView(result));
    } catch {
      setView((current) => ({
        status: "failed",
        currentVersion: current?.currentVersion ?? "",
        publicCode: "network_unavailable",
        retryable: true,
      }));
    }
  }, []);

  useNativeUpdateEvents(setView, setUiReady, checkNow);

  const dismiss = useCallback(() => setView(null), []);

  const recordDecision = useCallback(async (decision: "remind" | "skip") => {
    if (!view?.availableVersion) return;
    try {
      await invoke("record_application_update_decision", {
        version: view.availableVersion,
        decision,
      });
      setView(null);
    } catch {
      setView((current) => current ? { ...current, status: "failed", publicCode: "decision_failed", retryable: true } : current);
    }
  }, [view]);

  const install = useCallback(async () => {
    if (!view?.availableVersion) return;
    setView((current) => current ? { ...current, status: "downloading" } : current);
    try {
      const result = await invoke<ApplicationUpdateInstallEvent>(
        "install_pending_application_update",
        { version: view.availableVersion },
      );
      setView((current) => ({
        status: result.status,
        currentVersion: current?.currentVersion ?? "",
        availableVersion: current?.availableVersion,
        notes: current?.notes,
        fullNotesAvailable: current?.fullNotesAvailable,
        downloadedBytes: result.downloadedBytes,
        totalBytes: result.totalBytes,
        publicCode: result.publicCode,
        retryable: result.retryable,
      }));
    } catch {
      setView((current) => current ? { ...current, status: "failed", publicCode: "install_failed", retryable: true } : current);
    }
  }, [view]);

  const openFullNotes = useCallback(async () => {
    if (!view?.availableVersion) return;
    try {
      await invoke("open_application_update_release_notes", {
        version: view.availableVersion,
      });
    } catch {
      setView((current) => current ? { ...current, status: "failed", publicCode: "release_notes_failed", retryable: true } : current);
    }
  }, [view]);

  const restart = useCallback(() => {
    const proceed = () => void invoke("restart_after_application_update").catch(() => {
      setView((current) => current ? { ...current, status: "failed", publicCode: "restart_failed", retryable: true } : current);
    });
    if (navGuardRef.current) {
      navGuardRef.current(proceed);
    } else {
      proceed();
    }
  }, [navGuardRef]);

  return {
    view,
    uiReady,
    checkNow,
    dismiss,
    install,
    openFullNotes,
    remind: () => recordDecision("remind"),
    retry: () => ["download_failed", "signature_invalid", "install_failed"].includes(view?.publicCode ?? "")
      ? install()
      : checkNow(),
    restart,
    skip: () => recordDecision("skip"),
  };
}
