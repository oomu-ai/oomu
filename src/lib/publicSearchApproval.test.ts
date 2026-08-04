import { describe, expect, it } from "vitest";
import {
  chatMcpShieldApprovalRequest,
  mcpApprovalScopeFields,
  normalizedMcpScopeKind,
} from "./publicSearchApproval";

const publicSearch = {
  approvalToken: "search-token",
  approvalScopeKinds: ["once", "chat_session"],
  message: "Search the public web",
  serverName: "local_search",
  toolName: "search_web",
};

describe("public search approval presentation", () => {
  it("offers current-chat approval on the exact conversational search path", () => {
    expect(chatMcpShieldApprovalRequest(
      publicSearch,
      { generationToken: "generation", sessionId: "chat", turnId: "turn" },
      null,
      null,
    )).toMatchObject({
      actionType: "public_web_search",
      approvalScopeKinds: ["once", "chat_session"],
      scopeTrustAvailable: true,
      sessionId: "chat",
    });
  });

  it("keeps every other tool on one-use approval", () => {
    const otherTool = { ...publicSearch, serverName: "remote", toolName: "summarize" };
    expect(mcpApprovalScopeFields(otherTool, "chat")).toMatchObject({
      actionType: "mcp_tool_call",
      approvalScopeKinds: ["once"],
      scopeTrustAvailable: false,
    });
    expect(normalizedMcpScopeKind(otherTool, "chat_session")).toBe("once");
  });

  it("does not replace a native Apple approval presentation", () => {
    expect(chatMcpShieldApprovalRequest(
      publicSearch,
      { generationToken: "generation", sessionId: "chat", turnId: "turn" },
      { actionLabel: "Read Calendar", actionType: "read_calendar", preview: "{}" },
      null,
    )).toMatchObject({
      actionType: "read_calendar",
      approvalScopeKinds: ["once"],
      scopeTrustAvailable: false,
    });
  });
});
