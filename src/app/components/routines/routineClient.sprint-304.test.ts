import { describe, expect, it } from "vitest";
import { isBackgroundStatus, type BackgroundStatus } from "./routineClient";

function verifiedStatus(): BackgroundStatus {
  return {
    userEnabled: true,
    verifiedActive: true,
    state: "on_verified",
    registrationState: "registered",
    registrationBackend: "supervised_process",
    processState: "running",
    registrationGeneration: "7",
    processId: 42,
    buildNumber: 2,
    buildIdentity: "build-2",
    profileClass: "development",
    profileGenerationSha256: "profile-digest",
    heartbeatAtMs: 100,
    heartbeatAgeMs: 0,
    menuVisible: true,
    errorCode: null,
    detail: "verified",
    checkedAtMs: 100,
    recentReceipts: [],
  };
}

describe("Sprint 304 background evidence", () => {
  it("accepts On only when registration, worker heartbeat, and menu agree", () => {
    const verified = verifiedStatus();
    expect(isBackgroundStatus(verified)).toBe(true);
    for (const inconsistent of [
      { ...verified, verifiedActive: false },
      { ...verified, registrationState: "unregistered" },
      { ...verified, processState: "absent" },
      { ...verified, processId: null },
      { ...verified, heartbeatAtMs: null },
      { ...verified, menuVisible: false },
      { ...verified, errorCode: "background_runtime_worker_stopped" },
    ]) {
      expect(isBackgroundStatus(inconsistent)).toBe(false);
    }
  });
});
