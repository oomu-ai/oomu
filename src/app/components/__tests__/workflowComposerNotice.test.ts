import { describe, expect, it } from "vitest";
import { friendlyAuthoringError } from "../workflowComposerNotice";

describe("workflow composer notices", () => {
  it("localizes the bounded composer timeout instead of exposing an internal code", () => {
    expect(
      friendlyAuthoringError(
        "workflow_composer_timeout",
        (key: string) => key,
      ),
    ).toBe("workflows.composer.timeout_error");
  });

  it("localizes a structured composer timeout instead of exposing backend prose", () => {
    expect(
      friendlyAuthoringError(
        {
          code: "workflow_composer_timeout",
          message: "workflow worker exceeded the internal deadline",
        },
        (key: string) => key,
      ),
    ).toBe("workflows.composer.timeout_error");
  });
});
