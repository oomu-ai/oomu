import { describe, expect, it } from "vitest";

import { stableErrorCode } from "./inferenceErrors";

describe("stableErrorCode", () => {
  it("preserves stable codes thrown as local Error instances", () => {
    expect(stableErrorCode(new Error("auto_route_session_required")))
      .toBe("auto_route_session_required");
    expect(stableErrorCode(Object.assign(
      new Error("This objective is conversational."),
      { code: "agent_objective_not_executable" },
    ))).toBe("agent_objective_not_executable");
  });

  it("does not expose arbitrary exception messages as error codes", () => {
    expect(stableErrorCode(new Error("The database path was unavailable."))).toBe("");
  });
});
