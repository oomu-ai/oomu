import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  checkMacPermissionAndResumeForExecution,
  openMacPermissionSettings,
} from "./agentExecutionRecovery";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  isTauriRuntime: true,
}));

describe("Apple permission execution recovery", () => {
  beforeEach(() => invokeMock.mockReset());

  it("opens the exact settings destination for the stopped capability", async () => {
    invokeMock.mockResolvedValue(undefined);
    await expect(openMacPermissionSettings("contacts")).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("open_macos_permission_settings", {
      request: { capabilityId: "contacts" },
    });
  });

  it("checks current permission before resuming the exact stopped execution", async () => {
    invokeMock
      .mockResolvedValueOnce([{ capabilityId: "contacts", state: "allowed" }])
      .mockResolvedValueOnce({
        resumed: true,
        executionId: "execution-7",
        reason: "resumed",
      });
    await expect(checkMacPermissionAndResumeForExecution("execution-7", "contacts"))
      .resolves.toBe("resumed");
    expect(invokeMock).toHaveBeenNthCalledWith(2, "resume_agent_execution_after_permission", {
      request: { capabilityId: "contacts", executionId: "execution-7" },
    });
  });

  it("does not resume while permission is still denied", async () => {
    invokeMock.mockResolvedValue([{ capabilityId: "contacts", state: "denied" }]);
    await expect(checkMacPermissionAndResumeForExecution("execution-7", "contacts"))
      .rejects.toThrow("mac_permission_not_allowed");
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });
});
