import { describe, expect, it } from "vitest";
import {
  assistantControlProjection,
  conversationalMcpCapabilitiesFromServers,
  conversationalMcpToolIsAvailable,
} from "./conversationalMcpProtocol";

const translate = (key: string) => key === "chat.mcp.requesting_tool"
  ? "OOMU is using a connected tool…"
  : key;

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

  it("keeps authored assistant text after removing the control block", () => {
    const projection = assistantControlProjection(
      `I’ll check that file now.\n${toolRequest}`,
      translate,
    );

    expect(projection.displayText).toBe("I’ll check that file now.");
  });
});
