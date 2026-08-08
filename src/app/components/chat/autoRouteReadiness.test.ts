import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  currentSessionAutoRouteReadiness,
  normalizeAutoRouteSessionReadiness,
  normalizeLocalModelStatus,
  useAutoRouteReadiness,
} from "./autoRouteReadiness";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauriRuntime: true,
}));

const readySnapshot = {
  status: "ready",
  sessionId: "session-301",
  dynamicBindingValid: true,
  classifierModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
  classifierReady: true,
  localProviderId: "local-model-provider",
  localProviderType: "local_model",
  localModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
  routeGeneration: 4,
  localModelReady: true,
  recommendedLocalProviderId: null,
  recommendedLocalModelId: null,
  contextBudgetValid: true,
  cloudTargetRequired: false,
  cloudTargetReady: false,
  storageReady: true,
  auditReady: true,
  readinessGeneration: 4,
  lastVerifiedAtMs: 1_784_000_000_000,
  failureCode: null,
  failureBoundary: null,
};

beforeEach(() => {
  invokeMock.mockReset();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("current-session Auto-route readiness", () => {
  it("reports ready only when the complete current-session snapshot is valid", () => {
    expect(normalizeAutoRouteSessionReadiness(readySnapshot, "session-301"))
      .toMatchObject({ status: "ready", sessionId: "session-301" });

    for (const incomplete of [
      { classifierReady: false },
      { localModelReady: false },
      { dynamicBindingValid: false },
      { localProviderType: null },
      { routeGeneration: 0 },
      { contextBudgetValid: false },
      { storageReady: false },
      { auditReady: false },
      { readinessGeneration: 0 },
      { lastVerifiedAtMs: null },
    ]) {
      expect(normalizeAutoRouteSessionReadiness(
        { ...readySnapshot, ...incomplete },
        "session-301",
      ).status).toBe("degraded");
    }
  });

  it("rejects a fresh-looking snapshot from another session", () => {
    expect(normalizeAutoRouteSessionReadiness(readySnapshot, "session-new").status)
      .toBe("degraded");
  });

  it("does not expose the previous session while a new chat is hydrating", () => {
    const previous = normalizeAutoRouteSessionReadiness(readySnapshot, "session-301");
    expect(currentSessionAutoRouteReadiness(previous, "session-302", true)).toMatchObject({
      sessionId: "session-302",
      status: "loading",
      classifierReady: false,
    });
  });

  it("requires a real cloud target only when the native policy says it is needed", () => {
    expect(normalizeAutoRouteSessionReadiness({
      ...readySnapshot,
      cloudTargetRequired: true,
      cloudTargetReady: false,
    }, "session-301").status).toBe("degraded");
    expect(normalizeAutoRouteSessionReadiness({
      ...readySnapshot,
      cloudTargetRequired: true,
      cloudTargetReady: true,
    }, "session-301").status).toBe("ready");
  });

  it("fails closed on malformed native readiness and local-generation payloads", () => {
    expect(normalizeAutoRouteSessionReadiness(null, "session-301").status).toBe("unknown");
    expect(normalizeAutoRouteSessionReadiness({ ...readySnapshot, status: "excellent" }, "session-301").status)
      .toBe("unknown");
    expect(normalizeLocalModelStatus({ status: "ready" })).toBe("ready");
    expect(normalizeLocalModelStatus({ status: "excellent" })).toBe("unknown");
  });
});

describe("Auto-route readiness refresh scheduling", () => {
  it("never overlaps native readiness checks when one check is still running", async () => {
    vi.useFakeTimers();
    let resolvePoll: ((value: unknown) => void) | undefined;
    const pendingPoll = new Promise<unknown>((resolve) => {
      resolvePoll = resolve;
    });
    invokeMock.mockReturnValue(pendingPoll);

    renderHook(() => useAutoRouteReadiness({
      sessionId: "session-301",
      dynamicRoutingEnabled: true,
      localModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
    }));

    await act(async () => Promise.resolve());
    expect(invokeMock).toHaveBeenCalledTimes(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
      window.dispatchEvent(new Event("focus"));
    });
    expect(invokeMock).toHaveBeenCalledTimes(2);

    await act(async () => {
      resolvePoll?.(null);
      await Promise.resolve();
    });
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(invokeMock).toHaveBeenCalledTimes(4);
  });

  it("stops polling after native readiness is stable", async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation((command: string) => Promise.resolve(
      command === "get_local_generation_health" ? { status: "ready" } : readySnapshot,
    ));

    renderHook(() => useAutoRouteReadiness({
      sessionId: "session-301",
      dynamicRoutingEnabled: true,
      localModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
    }));

    await act(async () => Promise.resolve());
    expect(invokeMock).toHaveBeenCalledTimes(2);
    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(invokeMock).toHaveBeenCalledTimes(2);
  });

  it("rechecks transitional readiness and stops once it becomes stable", async () => {
    vi.useFakeTimers();
    let readinessChecks = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_local_generation_health") {
        return Promise.resolve({ status: readinessChecks === 0 ? "loading" : "ready" });
      }
      readinessChecks += 1;
      return Promise.resolve(readinessChecks === 1
        ? { ...readySnapshot, status: "loading", classifierReady: false }
        : readySnapshot);
    });

    renderHook(() => useAutoRouteReadiness({
      sessionId: "session-301",
      dynamicRoutingEnabled: true,
      localModelId: "gemma-4-E2B-it-qat-q4_0-gguf",
    }));

    await act(async () => Promise.resolve());
    expect(invokeMock).toHaveBeenCalledTimes(2);
    await act(async () => vi.advanceTimersByTimeAsync(5_000));
    expect(invokeMock).toHaveBeenCalledTimes(4);
    await act(async () => vi.advanceTimersByTimeAsync(30_000));
    expect(invokeMock).toHaveBeenCalledTimes(4);
  });
});
