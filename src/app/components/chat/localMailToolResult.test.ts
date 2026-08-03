import { describe, expect, it } from "vitest";
import {
  localMailFailureKey,
  localMailToolResultText,
} from "./localMailToolResult";

const copy = {
  toolFailureWithoutDetails: "Local tool failed.",
  toolResultMissing: "Local tool returned no result.",
};

describe("localMailToolResultText", () => {
  it("preserves bounded detail from a real native error envelope", () => {
    const resultText = localMailToolResultText({
      content: [{ type: "text", text: "Mail could not read the inbox." }],
      structuredContent: {
        warning: "execution_failed",
        error: "Mail could not read the inbox.",
        emails: [],
      },
      isError: true,
    }, copy);

    expect(resultText).toContain("execution_failed");
    expect(resultText).toContain("Mail could not read the inbox.");
  });

  it.each([
    ["mail_permission_denied", "permission"],
    ["AppleScript execution timed out after 5s.", "timeout"],
  ])("keeps %s on the recoverable path", (error, code) => {
    expect(() => localMailToolResultText({
      content: [{ type: "text", text: error }],
      structuredContent: { error, emails: [] },
      isError: true,
    }, copy)).toThrow(expect.objectContaining({ code }));
  });

  it("maps terminal failures to truthful Mail copy", () => {
    expect(localMailFailureKey({ code: "mail_unavailable" }))
      .toBe("chat.errors.mail_unavailable");
    expect(localMailFailureKey({ code: "execution_failed" }))
      .toBe("chat.errors.mail_failed");
  });
});
