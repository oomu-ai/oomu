"use client";

import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@/lib/invoke";

type SystemStatus =
  | "unknown"
  | "checking"
  | "ready"
  | "degraded"
  | "unavailable"
  | "error";

type AppContextType = {
  dbStatus: SystemStatus;
  networkStatus: SystemStatus;
  localModelStatus: SystemStatus;
  localModelWarm: boolean;
  isInitializing: boolean;
  isSecureEnvironment: boolean;
  refreshHealth: () => Promise<void>;
};

type NativeHealth = {
  subsystems: Array<{
    subsystem: string;
    active: boolean;
    backingStoreClass: "notApplicable" | "persistent" | "recoveryPending" | "volatile";
  }>;
};

type NativeNetworkReport = {
  state:
    | "not_checked"
    | "local_network_available"
    | "approved_endpoint_reachable"
    | "internet_not_established"
    | "failed";
  internetReachabilityVerified: boolean;
};

const persistenceSubsystems = new Set([
  "knowledge",
  "audit",
  "agent",
  "memory",
  "taskFlow",
  "chatSessionPersistence",
]);

const AppContext = createContext<AppContextType | null>(null);

function persistenceStatus(health: NativeHealth): SystemStatus {
  const persistence = health.subsystems.filter((entry) =>
    persistenceSubsystems.has(entry.subsystem),
  );
  if (persistence.length === 0) return "unavailable";
  return persistence.some(
    (entry) => entry.active || entry.backingStoreClass !== "persistent",
  )
    ? "degraded"
    : "ready";
}

function modelStatus(status: string): SystemStatus {
  switch (status.trim().toLowerCase()) {
    case "loading":
      return "checking";
    case "ready":
      return "ready";
    case "degraded":
      return "degraded";
    case "shutdown":
      return "unavailable";
    default:
      return "error";
  }
}

function networkStatus(report: NativeNetworkReport): SystemStatus {
  switch (report.state) {
    case "local_network_available":
    case "approved_endpoint_reachable":
      return "ready";
    case "internet_not_established":
      return "degraded";
    case "not_checked":
      return "unknown";
    case "failed":
      return "error";
  }
}

export function AppContextProvider({ children }: { children: ReactNode }) {
  const [dbStatus, setDbStatus] = useState<SystemStatus>("checking");
  const [nativeNetworkStatus, setNativeNetworkStatus] = useState<SystemStatus>("checking");
  const [localModelStatus, setLocalModelStatus] = useState<SystemStatus>("checking");

  const refreshHealth = useCallback(async () => {
    const [persistence, model, network] = await Promise.allSettled([
      invoke<NativeHealth>("get_degraded_mode_status"),
      invoke<string>("get_local_model_status"),
      invoke<NativeNetworkReport>("run_network_diagnostic", { request: null }),
    ]);
    setDbStatus(
      persistence.status === "fulfilled" ? persistenceStatus(persistence.value) : "error",
    );
    setLocalModelStatus(
      model.status === "fulfilled" ? modelStatus(model.value) : "error",
    );
    setNativeNetworkStatus(
      network.status === "fulfilled" ? networkStatus(network.value) : "error",
    );
  }, []);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    const refreshIfMounted = async () => {
      if (!cancelled) await refreshHealth();
    };
    void refreshIfMounted();
    const healthPoll = window.setInterval(() => void refreshIfMounted(), 5_000);

    async function subscribeToNativeHealthChanges() {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        for (const eventName of ["oomu://degraded-runtime", "token-stream"]) {
          const unlisten = await listen(eventName, () => void refreshIfMounted());
          if (cancelled) {
            unlisten();
          } else {
            unlisteners.push(unlisten);
          }
        }
      } catch {
        // Command failures above remain visible as `error`; browser previews do
        // not invent subsystem health when native events are unavailable.
      }
    }
    void subscribeToNativeHealthChanges();

    return () => {
      cancelled = true;
      window.clearInterval(healthPoll);
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [refreshHealth]);

  const value = useMemo<AppContextType>(() => {
    const isInitializing = [dbStatus, nativeNetworkStatus, localModelStatus].some(
      (status) => status === "unknown" || status === "checking",
    );
    return {
      dbStatus,
      networkStatus: nativeNetworkStatus,
      localModelStatus,
      localModelWarm: localModelStatus === "ready",
      isInitializing,
      // The lock badge represents verified encrypted persistence. Network or
      // model availability is reported independently and cannot confer it.
      isSecureEnvironment: dbStatus === "ready",
      refreshHealth,
    };
  }, [dbStatus, localModelStatus, nativeNetworkStatus, refreshHealth]);

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useAppContext() {
  const context = useContext(AppContext);
  if (!context) {
    throw new Error("useAppContext must be used within an AppContextProvider");
  }
  return context;
}
