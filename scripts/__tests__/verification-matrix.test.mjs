import { describe, expect, it } from "vitest";
import { classifyVerificationResult } from "../verification-matrix.mjs";

describe("verification result classification", () => {
  it("distinguishes passed, failed, environment-blocked, and not-run", () => {
    expect(classifyVerificationResult({ profile: "local", result: { status: 0 } })).toBe("passed");
    expect(classifyVerificationResult({ profile: "local", result: { status: 1, stderr: "assertion failed" } })).toBe("failed");
    expect(classifyVerificationResult({
      profile: "local",
      environmentSensitive: true,
      result: { status: 1, stderr: "binding to a port: Operation not permitted (os error 1)" },
    })).toBe("environment-blocked");
    expect(classifyVerificationResult({
      profile: "local",
      result: { status: null, error: { code: "ENOENT" } },
    })).toBe("not-run");
  });

  it("does not allow an environment block to satisfy a qualified run", () => {
    expect(classifyVerificationResult({
      profile: "qualified",
      environmentSensitive: true,
      result: { status: 1, stderr: "Operation not permitted (os error 1)" },
    })).toBe("failed");
  });
});
