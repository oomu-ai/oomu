import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import { SessionContextPanel, useSessionContextController } from "./SessionContextPanel";

vi.mock("@/lib/invoke", () => ({ invoke: vi.fn() }));

function localeState() {
  return { activeLocale: "en-US", availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }], translations: {} };
}

function Harness() {
  const controller = useSessionContextController({ refreshSignal: 0, sessionId: "session-1" });
  return <SessionContextPanel controller={controller} />;
}

describe("session Context panel", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_session_context_status") {
        return { estimatedTokensUsed: 6144, workingBudgetTokens: 12288, providerMaxTokens: 131072, estimatedPercentageUsed: 0.5, autoCompactionThresholdPercent: 70, autoCompactionEnabled: true };
      }
      if (command === "compact_chat_session") {
        return { sessionId: "session-1", beforeTokens: 6144, afterTokens: 3100, targetTokens: 8601, compactedMessageCount: 8, preservedMessageCount: 4, nextRequestTokens: 3200, thresholdPercent: 70 };
      }
      return null;
    });
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("is always visible with measured use, provider maximum, and an honest explanation", async () => {
    render(<Harness />, { wrapper: I18nProvider });
    expect(await screen.findByText("About 6,144 of 12,288 tokens in use")).toBeVisible();
    expect(screen.getByText("This model can accept up to 131,072 tokens.")).toBeVisible();
    expect(screen.getByText(/keeps your current work, decisions, files, and approvals/)).toBeVisible();
  });

  it("persists auto-compaction state and threshold through the backend policy", async () => {
    render(<Harness />, { wrapper: I18nProvider });
    const toggle = await screen.findByRole("switch", { name: "Compact automatically" });
    await waitFor(() => expect(toggle).toBeEnabled());
    fireEvent.click(toggle);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("save_session_context_policy", {
      request: { sessionId: "session-1", autoCompactionEnabled: false, autoCompactionThresholdPercent: 70 },
    }));
  });

  it("reports the measured next-request reduction after manual compaction", async () => {
    render(<Harness />, { wrapper: I18nProvider });
    const compact = await screen.findByRole("button", { name: "Compact now" });
    await waitFor(() => expect(compact).toBeEnabled());
    fireEvent.click(compact);
    expect(await screen.findByText("Estimated conversation history for the next request was reduced from 6,144 to 3,200 tokens.")).toBeVisible();
  });

  it("refreshes when state changes without polling encrypted storage while idle", async () => {
    vi.useFakeTimers();
    render(<Harness />, { wrapper: I18nProvider });
    await act(async () => vi.advanceTimersByTimeAsync(0));
    const initialChecks = vi.mocked(invoke).mock.calls
      .filter(([command]) => command === "get_session_context_status").length;
    expect(initialChecks).toBe(1);

    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(vi.mocked(invoke).mock.calls
      .filter(([command]) => command === "get_session_context_status")).toHaveLength(initialChecks);
  });
});
