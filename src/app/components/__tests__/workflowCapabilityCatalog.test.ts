import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  composeWorkflowFromNaturalLanguage,
  flattenCapabilityCatalog,
  loadWorkflowCapabilityCatalog,
  type CapabilityCatalog,
} from "../workflowCapabilityCatalog";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
}));

const nativeCatalog: CapabilityCatalog = {
  version: "2026-06-29.p2",
  authoringEnabled: true,
  generatedAtMs: 123,
  actions: [
    {
      id: "mcp:local_filesystem:read_file",
      kind: "mcp_tool",
      title: "Read a file",
      outcome: "Read a file",
      detail: "Native MCP capability",
      source: "mcp",
      available: true,
      availability: "available",
      serverName: "local_filesystem",
      toolName: "read_file",
    },
  ],
  templates: [],
};

describe("workflowCapabilityCatalog", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("loads only the capability catalog reported by the native runtime", async () => {
    const futureNativeCatalog = { ...nativeCatalog, version: "native-v99" };
    invokeMock.mockResolvedValue(futureNativeCatalog);

    await expect(loadWorkflowCapabilityCatalog()).resolves.toEqual(futureNativeCatalog);
    expect(invokeMock).toHaveBeenCalledWith("get_workflow_capability_catalog");
    expect(flattenCapabilityCatalog(futureNativeCatalog)).toEqual(nativeCatalog.actions);
  });

  it("does not synthesize capabilities when the native catalog fails", async () => {
    invokeMock.mockRejectedValue(new Error("registry unavailable"));

    await expect(loadWorkflowCapabilityCatalog()).rejects.toThrow("registry unavailable");
  });

  it("delegates compose requests when authoring is enabled by default", async () => {
    invokeMock.mockResolvedValue({
      status: "composed",
      reason: "Composed",
      workflowIr: null,
      partialDraft: null,
      missingCapabilities: [],
      attempts: 1,
      latencyMs: 0,
    });
    const response = await composeWorkflowFromNaturalLanguage({
      prompt: "Summarize my calendar.",
      catalog: nativeCatalog,
    });

    expect(response.status).toBe("composed");
    expect(invokeMock).toHaveBeenCalledWith("compose_workflow", {
      request: expect.objectContaining({
        capabilityCatalog: nativeCatalog,
        prompt: "Summarize my calendar.",
      }),
    });
  });
});
