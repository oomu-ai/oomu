import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauriRuntime: true,
}));

import { useVerifiedStartupModel } from "../verifiedStartupModel";

function health(status: "loading" | "ready") {
  return {
    status,
    requestedModelId: "E2B",
    classifierModelId: "E2B",
    readinessGeneration: 1,
    residencyGeneration: 1,
    verifiedResidencyGeneration: status === "ready" ? 1 : 0,
    lastVerifiedAtMs: status === "ready" ? 1_785_384_767_323 : null,
  };
}

afterEach(() => {
  vi.useRealTimers();
  invokeMock.mockReset();
});

describe("useVerifiedStartupModel", () => {
  it("follows classifier readiness after a slow startup and fails closed again if health degrades", async () => {
    vi.useFakeTimers();
    invokeMock
      .mockResolvedValueOnce(health("loading"))
      .mockResolvedValueOnce(health("ready"))
      .mockResolvedValueOnce(health("loading"));

    const view = renderHook(() => useVerifiedStartupModel(true, false));
    await act(async () => Promise.resolve());
    expect(view.result.current).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(view.result.current).toBe("E2B");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(view.result.current).toBeNull();
  });
});
