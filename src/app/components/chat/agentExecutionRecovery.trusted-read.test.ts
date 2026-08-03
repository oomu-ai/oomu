import { describe, expect, it, vi } from "vitest";
import { localizedAgentPlanSummary } from "./agentExecutionRecovery";

describe("trusted read-only recovery presentation", () => {
  it("does not tell users an automatic read is awaiting approval", () => {
    const translate = vi.fn((key: string) => key);
    const summary = localizedAgentPlanSummary(translate, {
      id: "plan-read",
      objective: "Inspect the current project.",
      steps: [{}],
      trusted_automatic_execution: true,
    });

    expect(summary).not.toContain("chat.recovery.plan_steps");
    expect(translate).not.toHaveBeenCalledWith(
      "chat.recovery.plan_steps",
      expect.anything(),
    );
  });
});
