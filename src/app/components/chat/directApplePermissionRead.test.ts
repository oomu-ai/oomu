import { describe, expect, it, vi } from "vitest";
import { runPermissionRecoverableAppleRead } from "./directApplePermissionRead";

describe("permission-recoverable Apple reads", () => {
  it("retries the same operation after permission recovery", async () => {
    const operation = vi.fn()
      .mockRejectedValueOnce({ code: "calendar_permission_denied" })
      .mockResolvedValueOnce("today's events");
    const recover = vi.fn(async () => "retry" as const);

    await expect(runPermissionRecoverableAppleRead(operation, recover)).resolves.toEqual({
      status: "completed",
      value: "today's events",
    });
    expect(operation).toHaveBeenCalledTimes(2);
    expect(recover).toHaveBeenCalledTimes(1);
  });

  it("ends without replaying when the user cancels", async () => {
    const operation = vi.fn().mockRejectedValue({ code: "mail_permission_denied" });

    await expect(runPermissionRecoverableAppleRead(
      operation,
      () => Promise.resolve("cancel"),
    )).resolves.toEqual({ status: "cancelled" });
    expect(operation).toHaveBeenCalledTimes(1);
  });

  it("preserves a non-permission failure", async () => {
    const failure = new Error("mailbox_unavailable");
    const operation = vi.fn().mockRejectedValue(failure);

    await expect(runPermissionRecoverableAppleRead(operation, () => null)).rejects.toBe(failure);
  });
});
