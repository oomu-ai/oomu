import { describe, expect, it } from "vitest";
import { shouldBlockUnverifiedActionClaim } from "./executionIntentPolicy";
import {
  mcpMutationResultHasVerifiedPostcondition,
  nativeMcpExecutionReceipt,
  verifiedSovereignMcpSearchResult,
} from "./mcpToolResults";

const sovereignSearchDigest = "a".repeat(64);
const sovereignSearchQuery = "Writing AI Prompts for Dummies latest edition";
const sovereignSearchEngine = "duckduckgo_lite_static";
const sovereignSearchContext = JSON.stringify({
  accessedAtUtc: "2026-08-01T20:30:00.000Z",
  pages: [{ url: "https://www.wiley.com/en-us/Writing+AI+Prompts+For+Dummies-p-9781394283126" }],
});

function sovereignSearchResult() {
  return {
    content: [{ type: "text", text: "Verified public search result." }],
    structuredContent: {
      sovereignSearch: {
        query: sovereignSearchQuery,
        engine: sovereignSearchEngine,
        resultCount: 1,
        contextJson: sovereignSearchContext,
        degraded: false,
        receiptDigest: sovereignSearchDigest,
        invocationIndex: 1,
      },
    },
    isError: false,
    _meta: {
      oomuSovereignSearchReceipt: {
        schema: "oomu.sovereign-mcp-search.v1",
        verified: true,
        query: sovereignSearchQuery,
        engine: sovereignSearchEngine,
        resultCount: 1,
        receiptDigest: sovereignSearchDigest,
        invocationIndex: 1,
      },
    },
  };
}

describe("native MCP execution authority", () => {
  it("reads the native-authored receipt projection without inferring authority from content", () => {
    expect(nativeMcpExecutionReceipt({
      content: [{ type: "text", text: "done" }],
      isError: false,
      _meta: {
        oomuNativeExecutionReceipt: {
          schema: "oomu.native-mcp-execution.v1",
          receiptId: "apple-operation-1-abcdef",
          outcome: "succeeded",
          verified: true,
          postcondition: { nativeResultCode: "verified" },
        },
      },
    })).toEqual({
      receiptId: "apple-operation-1-abcdef",
      outcome: "succeeded",
      verified: true,
      nativeResultCode: "verified",
    });
  });

  it("rejects a nominal result without the native receipt schema", () => {
    expect(nativeMcpExecutionReceipt({
      content: [{ type: "text", text: "I promise this worked" }],
      structuredContent: { verified: true },
      isError: false,
    })).toBeNull();
  });
});

describe("verified sovereign MCP search results", () => {
  it("accepts only a complete result bound to the exact native receipt", () => {
    expect(verifiedSovereignMcpSearchResult(sovereignSearchResult())).toEqual({
      query: sovereignSearchQuery,
      engine: sovereignSearchEngine,
      resultCount: 1,
      contextJson: sovereignSearchContext,
      degraded: false,
      receiptDigest: sovereignSearchDigest,
      invocationIndex: 1,
    });
  });

  it("rejects a nominal search result with no sovereign receipt marker", () => {
    const result = sovereignSearchResult();

    expect(verifiedSovereignMcpSearchResult({ ...result, _meta: {} })).toBeNull();
  });

  it.each([
    ["query", "a different query"],
    ["engine", "unverified_engine"],
    ["resultCount", 2],
    ["receiptDigest", "b".repeat(64)],
    ["invocationIndex", 2],
  ])("rejects a receipt whose %s does not match the structured result", (field, value) => {
    const result = sovereignSearchResult();
    Object.assign(result._meta.oomuSovereignSearchReceipt, { [field]: value });

    expect(verifiedSovereignMcpSearchResult(result)).toBeNull();
  });

  it.each([
    { degraded: true },
    { resultCount: 0 },
    { contextJson: "" },
    { contextJson: "not json" },
    { contextJson: JSON.stringify({ accessedAtUtc: "2026-08-01T20:30:00.000Z", pages: [] }) },
  ])("rejects incomplete or degraded public evidence: %o", (override) => {
    const result = sovereignSearchResult();
    Object.assign(result.structuredContent.sovereignSearch, override);

    expect(verifiedSovereignMcpSearchResult(result)).toBeNull();
  });
});

describe("verified MCP mutation receipts", () => {
  it("accepts an observed and verified local write postcondition", () => {
    const verified = mcpMutationResultHasVerifiedPostcondition(
      "local_filesystem",
      "write_file",
      {
        content: [{ type: "text", text: "Execution Completed: notes.txt generated successfully." }],
        structuredContent: {
          path: "notes.txt",
          relativePath: "notes.txt",
          bytesWritten: 7,
        },
        isError: false,
      },
    );

    expect(verified).toBe(true);
    expect(
      shouldBlockUnverifiedActionClaim(
        "I've written the requested update to notes.txt.",
        "Update my local notes file.",
        true,
        verified,
      ),
    ).toBe(false);
  });

  it("keeps a resolved MCP failure result behind the action-claim gate", () => {
    const verified = mcpMutationResultHasVerifiedPostcondition(
      "local_filesystem",
      "write_file",
      {
        content: [{ type: "text", text: "write_failed" }],
        structuredContent: {
          path: "notes.txt",
          bytesWritten: 7,
        },
        isError: true,
      },
    );

    expect(verified).toBe(false);
    expect(
      shouldBlockUnverifiedActionClaim(
        "I've written the requested update to notes.txt.",
        "Update my local notes file.",
        true,
        verified,
      ),
    ).toBe(true);
  });

  it("rejects a nominal success that has no verified mutation postcondition", () => {
    const verified = mcpMutationResultHasVerifiedPostcondition(
      "local_filesystem",
      "write_file",
      {
        content: [{ type: "text", text: "Write request accepted." }],
        structuredContent: { path: "notes.txt" },
        isError: false,
      },
    );

    expect(verified).toBe(false);
    expect(
      shouldBlockUnverifiedActionClaim(
        "I've written the requested update to notes.txt.",
        "Update my local notes file.",
        true,
        verified,
      ),
    ).toBe(true);
  });

  it("never treats a zero-byte write as a verified content mutation", () => {
    const verified = mcpMutationResultHasVerifiedPostcondition(
      "local_filesystem",
      "write_file",
      {
        content: [{ type: "text", text: "Execution Completed: report.md generated successfully." }],
        structuredContent: {
          path: "report.md",
          relativePath: "report.md",
          bytesWritten: 0,
        },
        isError: false,
      },
    );

    expect(verified).toBe(false);
    expect(
      shouldBlockUnverifiedActionClaim(
        "I've written the requested report to report.md.",
        "Research the subject and create a report.",
        true,
        verified,
      ),
    ).toBe(true);
  });

  it("accepts an explicitly approved empty file only with read-back existence and identity proof", () => {
    const receipt = {
      content: [{ type: "text" as const, text: "Execution Completed: placeholder.txt generated successfully." }],
      structuredContent: {
        path: "placeholder.txt",
        relativePath: "placeholder.txt",
        bytesWritten: 0,
        exists: true,
        verified: true,
        contentSha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        targetIdentityVerified: true,
      },
      isError: false,
    };

    expect(
      mcpMutationResultHasVerifiedPostcondition(
        "local_filesystem",
        "write_file",
        receipt,
        { path: "placeholder.txt", content: "" },
      ),
    ).toBe(true);
    expect(
      mcpMutationResultHasVerifiedPostcondition(
        "local_filesystem",
        "write_file",
        receipt,
        { path: "report.md", content: "" },
      ),
    ).toBe(false);
    expect(
      mcpMutationResultHasVerifiedPostcondition(
        "local_filesystem",
        "write_file",
        receipt,
        { path: "placeholder.txt", content: "expected report content" },
      ),
    ).toBe(false);
    expect(
      mcpMutationResultHasVerifiedPostcondition(
        "local_filesystem",
        "write_file",
        {
          ...receipt,
          structuredContent: {
            ...receipt.structuredContent,
            targetIdentityVerified: false,
          },
        },
        { path: "placeholder.txt", content: "" },
      ),
    ).toBe(false);
  });

  it("never promotes a read-only tool result to a mutation receipt", () => {
    expect(
      mcpMutationResultHasVerifiedPostcondition(
        "local_filesystem",
        "read_file",
        {
          content: [{ type: "text", text: "Existing contents" }],
          structuredContent: {
            path: "notes.txt",
            content: "Existing contents",
          },
          isError: false,
        },
      ),
    ).toBe(false);
  });

  it("requires a saved, read-back Mail draft identifier", () => {
    const nominal = {
      content: [{ type: "text" as const, text: "true" }],
      structuredContent: { success: true, subject: "Supplier Decision Review" },
      isError: false,
    };
    expect(
      mcpMutationResultHasVerifiedPostcondition(
        "macos_applescript",
        "draft_system_email",
        nominal,
      ),
    ).toBe(false);

    expect(
      mcpMutationResultHasVerifiedPostcondition(
        "macos_applescript",
        "draft_system_email",
        {
          ...nominal,
          structuredContent: {
            success: true,
            subject: "Supplier Decision Review",
            draftId: "draft-42",
            saved: true,
            verified: true,
          },
        },
      ),
    ).toBe(true);
  });
});
