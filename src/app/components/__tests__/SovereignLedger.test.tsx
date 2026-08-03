import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { SovereignLedger } from "../SovereignLedger";

const invokeMock = vi.hoisted(() => vi.fn());
const eventHarness = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: Record<string, unknown> }) => void>(),
  unlistenCalls: 0,
}));

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    async (
      eventName: string,
      callback: (event: { payload: Record<string, unknown> }) => void,
    ) => {
      eventHarness.listeners.set(eventName, callback);
      return () => {
        eventHarness.unlistenCalls += 1;
        eventHarness.listeners.delete(eventName);
      };
    },
  ),
}));

type TestStats = Record<string, number>;

const localeState = {
  activeLocale: "en-US",
  availableLocales: [
    {
      id: "en-US",
      label: "English (US)",
      fileName: "en-US.json",
      isDefault: true,
      verified: true,
    },
  ],
  translations: {},
};

let currentStats: TestStats;

function stats(outputTokens: number): TestStats {
  return {
    totalLocalTurns: 1,
    totalCloudTurns: 0,
    ratioOnDevice: 100,
    dataEgressProtectedMb: 0.1,
    protectedInputTokens: 0,
    protectedOutputTokens: outputTokens,
  };
}

function ledgerLoadCount() {
  return invokeMock.mock.calls.filter(
    ([command]) => command === "get_sovereign_ledger_stats",
  ).length;
}

function renderLedger() {
  return render(
    <I18nProvider>
      <SovereignLedger />
    </I18nProvider>,
  );
}

async function flushAsyncWork() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("SovereignLedger", () => {
  beforeEach(() => {
    vi.useRealTimers();
    currentStats = stats(180_000);
    eventHarness.listeners.clear();
    eventHarness.unlistenCalls = 0;
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") {
        return localeState;
      }
      if (command === "get_sovereign_ledger_stats") {
        return { ...currentStats };
      }
      return undefined;
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it.each([
    ["zero", 0, "$0.00"],
    ["a positive amount below half a cent", 1, "< $0.01"],
    ["sub-dollar savings", 180_000, "≈ $0.90"],
    ["larger estimates", 20_000_000, "≈ $100"],
  ])("formats %s without hiding meaningful small changes", async (_label, tokens, expected) => {
    currentStats = stats(tokens as number);
    renderLedger();

    expect(await screen.findByText(expected)).toBeVisible();
  });

  it("coalesces completion activity and renders changed raw stats", async () => {
    vi.useFakeTimers();
    currentStats = stats(100_000);
    renderLedger();
    await flushAsyncWork();

    expect(screen.getByText("≈ $0.50")).toBeVisible();
    expect(ledgerLoadCount()).toBe(1);

    currentStats = {
      total_local_turns: 2,
      total_cloud_turns: 0,
      ratio_on_device: 100,
      data_egress_protected_mb: 0.2,
      protected_input_tokens: 0,
      protected_output_tokens: 180_000,
    };
    const chatActivity = eventHarness.listeners.get("chat://token");
    expect(chatActivity).toBeDefined();
    chatActivity?.({ payload: { token: "one" } });
    chatActivity?.({ payload: { token: "two" } });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_499);
    });
    expect(ledgerLoadCount()).toBe(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(ledgerLoadCount()).toBe(2);
    expect(screen.getByText("≈ $0.90")).toBeVisible();
    expect(screen.getByText("2 of 2 replies ran on your device.")).toBeVisible();
  });

  it("uses a bounded fallback poll and cleans up timers and event listeners", async () => {
    vi.useFakeTimers();
    const view = renderLedger();
    await flushAsyncWork();
    expect(ledgerLoadCount()).toBe(1);

    currentStats = stats(190_000);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(ledgerLoadCount()).toBe(2);
    expect(screen.getByText("≈ $0.95")).toBeVisible();

    view.unmount();
    expect(eventHarness.unlistenCalls).toBe(4);
    const callsAtUnmount = ledgerLoadCount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(ledgerLoadCount()).toBe(callsAtUnmount);
  });
});
