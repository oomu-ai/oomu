import { describe, expect, it, vi } from "vitest";
import { actionPlanStepPresentation, type ActionPlanStep } from "./actionPlanPresentation";

const destination = "/Users/example/Desktop/OOMU reports/oomu-test.md";

function createFileStep(): ActionPlanStep {
  return {
    step: `Create Markdown file from assistant message 483 (${"a".repeat(64)}).`,
    tool: {
      kind: "registered_task_tool",
      operation: "create_file",
      arguments: { file: { destinationPath: destination } },
    },
    risk_level: "high",
  };
}

describe("action plan step presentation", () => {
  it("presents a registered create-file destination without lineage internals", () => {
    const step = createFileStep();
    const translate = vi.fn((_key: string, variables?: Record<string, string | number>) =>
      `Create a new file at ${variables?.path}.`);

    const presented = actionPlanStepPresentation(step, translate);

    expect(translate).toHaveBeenCalledWith(
      "chat.plan.create_file_destination",
      { path: destination },
    );
    expect(presented).toBe(`Create a new file at ${destination}.`);
    expect(presented).not.toContain("483");
    expect(presented).not.toContain("a".repeat(64));
  });

  it("preserves existing text for every other plan step", () => {
    const step: ActionPlanStep = {
      step: "Read the selected source file.",
      tool: { kind: "file_read", path: "/private/tmp/source.txt" },
      risk_level: "low",
    };

    expect(actionPlanStepPresentation(step, vi.fn())).toBe(step.step);
  });

  it("falls back safely when a create-file destination is unavailable", () => {
    const step = createFileStep();
    step.tool.arguments = { file: {} };

    expect(actionPlanStepPresentation(step, vi.fn())).toBe(step.step);
  });
});
