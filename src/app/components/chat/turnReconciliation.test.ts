import { describe, expect, it, vi } from "vitest";
import {
  hasTerminalChatTurnResult,
  waitForTerminalChatTurnResult,
} from "./turnReconciliation";

const terminalMessages = [
  {
    id: 2,
    sessionId: "session-1",
    role: "assistant" as const,
    content: "Finished safely.",
    metadataJson: JSON.stringify({ terminalResultForTurnId: "turn-1" }),
    createdAtMs: 2,
  },
];

describe("terminal chat turn reconciliation", () => {
  it("recognizes only the exact terminal turn receipt", () => {
    expect(hasTerminalChatTurnResult(terminalMessages, "turn-1")).toBe(true);
    expect(hasTerminalChatTurnResult(terminalMessages, "turn-2")).toBe(false);
  });

  it("waits through an early empty read and returns the durable reply", async () => {
    const fetchMessages = vi
      .fn<() => Promise<typeof terminalMessages>>()
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce(terminalMessages);

    await expect(
      waitForTerminalChatTurnResult(fetchMessages, "turn-1", {
        delaysMs: [0, 0],
      }),
    ).resolves.toEqual({ status: "terminal", messages: terminalMessages });
    expect(fetchMessages).toHaveBeenCalledTimes(2);
  });

  it("reports a bounded timeout without inventing a terminal result", async () => {
    const fetchMessages = vi.fn(async () => []);
    await expect(
      waitForTerminalChatTurnResult(fetchMessages, "turn-1", { delaysMs: [0, 0] }),
    ).resolves.toEqual({ status: "timed_out" });
    expect(fetchMessages).toHaveBeenCalledTimes(2);
  });

  it("aborts before another hydration read", async () => {
    const controller = new AbortController();
    controller.abort();
    const fetchMessages = vi.fn(async () => terminalMessages);
    await expect(
      waitForTerminalChatTurnResult(fetchMessages, "turn-1", {
        delaysMs: [1_000],
        signal: controller.signal,
      }),
    ).resolves.toEqual({ status: "cancelled" });
    expect(fetchMessages).not.toHaveBeenCalled();
  });
});
