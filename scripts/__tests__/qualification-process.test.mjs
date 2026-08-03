import { describe, expect, it, vi } from "vitest";
import {
  parseProcessSnapshot,
  sameOwnedProcess,
  sameProcessLifetime,
  stopOwnedProcess,
} from "../qualification-process.mjs";

function binding(overrides = {}) {
  return {
    pid: 4321,
    parentPid: 1234,
    processGroupId: 1200,
    launchTime: "Wed Jul 30 12:00:00 2026",
    executable: "/test/oomu",
    executableIdentitySha256: "a".repeat(64),
    loadedImageIdentitySha256: "c".repeat(64),
    executableSha256: "b".repeat(64),
    ...overrides,
  };
}

describe("exact qualification-process cleanup", () => {
  it("treats a dead zombie as exited instead of a reused live PID", () => {
    const running = "4321 1234 1200 S+ Wed Jul 30 12:00:00 2026 /test/oomu";
    const zombie = "4321 1 1200 Z Wed Jul 30 12:00:00 2026 (oomu)";
    expect(parseProcessSnapshot(running)).toMatchObject({
      pid: 4321,
      parentPid: 1234,
      processGroupId: 1200,
      processState: "S+",
      command: "/test/oomu",
    });
    expect(parseProcessSnapshot(zombie)).toBeNull();
  });

  it("matches stable process identity after macOS reparents an app", () => {
    const owned = binding();
    expect(sameOwnedProcess(owned, { ...owned })).toBe(true);
    expect(sameOwnedProcess(owned, binding({ parentPid: 1 }))).toBe(true);
    expect(sameOwnedProcess(owned, binding({ processGroupId: 1201 }))).toBe(false);
    expect(sameOwnedProcess(owned, binding({ launchTime: "Wed Jul 30 12:00:01 2026" })))
      .toBe(false);
    expect(sameOwnedProcess(owned, binding({ executableIdentitySha256: "b".repeat(64) })))
      .toBe(true);
    expect(sameOwnedProcess(owned, binding({ executableSha256: "d".repeat(64) })))
      .toBe(true);
    expect(sameOwnedProcess(owned, binding({ loadedImageIdentitySha256: "d".repeat(64) })))
      .toBe(false);
  });

  it("tracks the same running lifetime after an installed app path is replaced", () => {
    const owned = binding();
    const replacedOnDisk = binding({ executableSha256: "d".repeat(64) });
    expect(sameOwnedProcess(owned, replacedOnDisk)).toBe(true);
    expect(sameProcessLifetime(owned, replacedOnDisk)).toBe(true);
    expect(sameProcessLifetime(owned, binding({ parentPid: 1 }))).toBe(true);
    expect(sameProcessLifetime(owned, binding({
      launchTime: "Wed Jul 30 12:00:01 2026",
    }))).toBe(false);
  });

  it("uses one graceful exact-PID signal and verifies exit", async () => {
    const owned = binding();
    let observations = 0;
    const inspect = () => (++observations === 1 ? owned : null);
    const signal = vi.fn();

    const receipt = await stopOwnedProcess(owned, {
      inspect,
      signal,
      gracefulTimeoutMs: 10,
      pollMs: 1,
    });

    expect(signal).toHaveBeenCalledOnce();
    expect(signal).toHaveBeenCalledWith(owned.pid, "SIGTERM");
    expect(receipt).toMatchObject({
      kind: "exact_process_cleanup",
      status: "passed",
      synthetic: false,
      outcome: "graceful",
      forced: false,
      exitVerified: true,
    });
  });

  it("recognizes the same process lifetime while its loaded image is disappearing", async () => {
    const owned = binding();
    const exiting = binding({ loadedImageIdentitySha256: "d".repeat(64) });
    const observations = [owned, exiting, null];
    const signal = vi.fn();

    const receipt = await stopOwnedProcess(owned, {
      inspect: () => observations.shift() ?? null,
      signal,
      gracefulTimeoutMs: 10,
      pollMs: 1,
    });

    expect(signal).toHaveBeenCalledOnce();
    expect(receipt.outcome).toBe("graceful");
  });

  it("never signals a reused PID", async () => {
    const owned = binding();
    const reused = binding({ launchTime: "Wed Jul 30 13:00:00 2026" });
    const signal = vi.fn();

    await expect(stopOwnedProcess(owned, { inspect: () => reused, signal }))
      .rejects.toThrow(/reused or mismatched PID/iu);
    expect(signal).not.toHaveBeenCalled();
  });
});
