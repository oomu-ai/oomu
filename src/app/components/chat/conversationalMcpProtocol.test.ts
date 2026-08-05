import { describe, expect, it } from "vitest";
import {
  assistantControlProjection,
  conversationalMcpCapabilitiesFromServers,
  conversationalMcpToolIsAvailable,
  sanitizeAssistantTranscriptText,
} from "./conversationalMcpProtocol";

const translations: Record<string, string> = {
  "chat.errors.provider_response.content":
    "The provider returned an empty or unusable response. Try again or choose another model.",
  "chat.mcp.requesting_tool": "OOMU is using a connected tool…",
  "chat.status.searching_web": "Searching the web...",
};
const translate = (key: string) => translations[key] ?? key;

const toolRequest = [
  "```oomu_mcp_tool_call",
  JSON.stringify({
    server: "local_filesystem",
    name: "read_file",
    arguments: { path: "/tmp/example.txt" },
  }),
  "```",
].join("\n");

describe("assistantControlProjection", () => {
  it("projects every live native catalog schema without a renderer allowlist", () => {
    const capabilities = conversationalMcpCapabilitiesFromServers({
      google_workspace: {
        name: "google_workspace",
        status: "connected",
        tools: [{
          name: "calendar_list",
          description: "Read Google Calendar",
          inputSchema: { type: "object" },
        }],
      },
      local_search: {
        name: "local_search",
        status: "connected",
        tools: [{
          name: "search_web",
          description: "Search public sources",
          inputSchema: { type: "object" },
        }],
      },
      offline: {
        name: "offline",
        status: "disconnected",
        tools: [{ name: "hidden", description: "Hidden", inputSchema: {} }],
      },
    });

    expect(capabilities.map(({ serverName, toolName }) => `${serverName}/${toolName}`)).toEqual([
      "google_workspace/calendar_list",
      "local_search/search_web",
    ]);
    expect(conversationalMcpToolIsAvailable({
      serverName: "local_search",
      toolName: "search_web",
      argumentsValue: { query: "public facts" },
    }, capabilities)).toBe(true);
    expect(conversationalMcpToolIsAvailable({
      serverName: "local_filesystem",
      toolName: "read_file",
      argumentsValue: { path: "private.txt" },
    }, capabilities)).toBe(false);
  });

  it("uses localized human copy when a tool request has no assistant text", () => {
    const projection = assistantControlProjection(toolRequest, translate);

    expect(projection.mcpRequest).not.toBeNull();
    expect(projection.displayText).toBe("OOMU is using a connected tool…");
    expect(projection.displayText).not.toContain("local_filesystem");
    expect(projection.displayText).not.toContain("read_file");
  });

  it("shows localized progress while a control-only search continues", () => {
    const searchRequest = [
      "```oomu_search_request",
      JSON.stringify({ query: "affordable hotels in Hershey PA for Sept 15" }),
      "```",
    ].join("\n");
    const projection = assistantControlProjection(searchRequest, translate);

    expect(projection.searchRequest?.query).toBe(
      "affordable hotels in Hershey PA for Sept 15",
    );
    expect(projection.displayText).toBe("Searching the web...");
    expect(assistantControlProjection(
      `I’ll check current hotel listings.\n${searchRequest}`,
      translate,
    ).displayText).toBe("I’ll check current hotel listings.");
  });

  it("never projects an unusable terminal response as a blank message", () => {
    expect(assistantControlProjection("", translate).displayText).toBe(
      translations["chat.errors.provider_response.content"],
    );
    expect(sanitizeAssistantTranscriptText("<|tool_call>", translate)).toBe(
      translations["chat.errors.provider_response.content"],
    );
  });
});

describe("assistantControlProjection parsing", () => {

  it("keeps authored assistant text after removing the control block", () => {
    const projection = assistantControlProjection(
      `I’ll check that file now.\n${toolRequest}`,
      translate,
    );

    expect(projection.displayText).toBe("I’ll check that file now.");
  });

  it("recovers the exact compact request emitted by a small local model", () => {
    const projection = assistantControlProjection(
      "call:macos_applescript/read_system_contacts{search_text: OOMU Permission Probe 1785858001}",
      translate,
    );

    expect(projection.mcpRequest?.call).toEqual({
      serverName: "macos_applescript",
      toolName: "read_system_contacts",
      argumentsValue: { search_text: "OOMU Permission Probe 1785858001" },
    });
    expect(projection.displayText).toBe("OOMU is using a connected tool…");
  });

  it("never promotes compact-call text embedded in assistant prose", () => {
    const text = [
      "I would use this syntax:",
      "call:macos_applescript/read_system_contacts{search_text: someone}",
    ].join("\n");
    const projection = assistantControlProjection(text, translate);

    expect(projection.mcpRequest).toBeNull();
    expect(projection.displayText).toBe(text);
  });

  it("rejects nested or malformed compact arguments", () => {
    const projection = assistantControlProjection(
      "call:macos_applescript/read_system_contacts{search_text: {nested: value}}",
      translate,
    );

    expect(projection.mcpRequest).toBeNull();
  });
});
