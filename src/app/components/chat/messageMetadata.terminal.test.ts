import { describe, expect, it } from "vitest";
import {
  localizedAssistantTerminalContent,
  markAcceptedTurnTerminal,
  markAcceptedTurnTerminalAfterError,
  normalizeChatMessageMetadata,
} from "./messageMetadata";

describe("terminal chat metadata", () => {
  it("localizes a durable failure without exposing its internal boundary", () => {
    const metadata = normalizeChatMessageMetadata({
      terminal_error_code: "delete_target_not_found",
      terminal_error_boundary: "/Users/private/path/to/delete",
    });
    expect(metadata).toMatchObject({
      terminalErrorCode: "delete_target_not_found",
      terminalErrorBoundary: "/Users/private/path/to/delete",
    });
    const localized = localizedAssistantTerminalContent(
      "The chat request failed before OOMU could finish the response.", metadata,
      (key) => key === "chat.errors.delete_target_not_found.content"
        ? "That file is not there, so there is nothing to delete."
        : key,
    );
    expect(localized).toBe("That file is not there, so there is nothing to delete.");
    expect(localized).not.toContain("/Users/private");
  });

  it("clears the accepted state as soon as its turn fails", () => {
    expect(markAcceptedTurnTerminal([
      { role: "user", isPending: true, metadata: { turnId: "turn-1", turnState: "accepted" } },
      { role: "user", metadata: { turnId: "turn-2", turnState: "accepted" } },
    ], "turn-1", "failed")).toEqual([
      { role: "user", isPending: false, metadata: { turnId: "turn-1", turnState: "failed" } },
      { role: "user", metadata: { turnId: "turn-2", turnState: "accepted" } },
    ]);
  });

  it("leaves an accepted turn visible while its already-running owner reconciles", () => {
    const messages = [{ role: "user", metadata: { turnId: "turn-1", turnState: "accepted" } }];
    expect(markAcceptedTurnTerminalAfterError(messages, "turn-1", "chat_turn_already_running"))
      .toBe(messages);
  });
});
