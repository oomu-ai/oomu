import { describe, expect, it } from "vitest";
import {
  localSearchFailureCode,
  localSearchTerminalStatus,
} from "./localSearchErrors";

describe("localSearchFailureCode", () => {
  it("preserves stable codes from object and stringified Tauri rejections", () => {
    expect(localSearchFailureCode({ code: "search_provider_challenge" })).toBe(
      "search_provider_challenge",
    );
    expect(
      localSearchFailureCode(JSON.stringify({ code: "search_retrieval_timeout" })),
    ).toBe("search_retrieval_timeout");
    expect(
      localSearchFailureCode({ error: JSON.stringify({ errorCode: "search_dom_failed" }) }),
    ).toBe("search_dom_failed");
  });

  it("fails closed for malformed, oversized, or unknown error payloads", () => {
    expect(localSearchFailureCode("not-json")).toBe("search_unavailable");
    expect(localSearchFailureCode({ code: "provider_secret_500" })).toBe(
      "search_unavailable",
    );
    expect(localSearchFailureCode(`{"code":"${"x".repeat(16_001)}"}`)).toBe(
      "search_unavailable",
    );
  });
});

describe("localSearchTerminalStatus", () => {
  it("renders honest no-evidence and invalid-query outcomes as completed", () => {
    expect(localSearchTerminalStatus("search_no_results")).toBe("completed");
    expect(localSearchTerminalStatus("search_query_invalid")).toBe("completed");
  });

  it("keeps cancellation and operational failures distinct", () => {
    expect(localSearchTerminalStatus("search_cancelled")).toBe("cancelled");
    expect(localSearchTerminalStatus("search_provider_unavailable")).toBe("failed");
    expect(localSearchTerminalStatus("search_retrieval_timeout")).toBe("failed");
    expect(localSearchTerminalStatus(undefined)).toBe("failed");
  });
});
