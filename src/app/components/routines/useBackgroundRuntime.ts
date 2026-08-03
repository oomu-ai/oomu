import { useCallback, useEffect, useRef, useState } from "react";
import { isBackgroundStatus, routineApi, type BackgroundStatus } from "./routineClient";
import type { RoutineTranslate } from "./routineLabels";

export function useBackgroundRuntime(t: RoutineTranslate) {
  const [status, setStatus] = useState<BackgroundStatus | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const requestInFlight = useRef(false);

  const refresh = useCallback(async () => {
    if (requestInFlight.current) return;
    requestInFlight.current = true;
    setBusy(true);
    setError("");
    try {
      const nextStatus = await routineApi.background();
      if (!isBackgroundStatus(nextStatus)) throw new Error("background_status_invalid");
      setStatus(nextStatus);
    } catch {
      setError(t("routines.background_check_failed"));
    } finally {
      requestInFlight.current = false;
      setBusy(false);
    }
  }, [t]);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<BackgroundStatus>("oomu://background-runtime-status", (event) => {
          if (active && isBackgroundStatus(event.payload)) {
            setStatus(event.payload);
            setError("");
          }
        }),
      )
      .then((dispose) => {
        if (active) unlisten = dispose;
        else dispose();
      })
      .catch(() => undefined);
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => void refresh(), 0);
    return () => window.clearTimeout(timer);
  }, [refresh]);

  const setEnabled = useCallback(
    async (enabled: boolean) => {
      if (requestInFlight.current) return;
      requestInFlight.current = true;
      setBusy(true);
      setError("");
      try {
        const nextStatus = await routineApi.setBackground(enabled);
        if (!isBackgroundStatus(nextStatus)) throw new Error("background_status_invalid");
        setStatus(nextStatus);
      } catch {
        setError(t("routines.background_action_failed"));
      } finally {
        requestInFlight.current = false;
        setBusy(false);
      }
    },
    [t],
  );

  const openLoginItems = useCallback(async () => {
    if (requestInFlight.current) return;
    requestInFlight.current = true;
    setBusy(true);
    setError("");
    try {
      await routineApi.openBackgroundLoginItems();
    } catch {
      setError(t("routines.background_settings_failed"));
    } finally {
      requestInFlight.current = false;
      setBusy(false);
    }
  }, [t]);

  return { busy, error, openLoginItems, refresh, setEnabled, status };
}
