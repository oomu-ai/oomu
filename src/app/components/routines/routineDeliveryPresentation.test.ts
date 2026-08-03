import { describe, expect, it } from "vitest";
import { shouldShowRoutinePausedReason } from "./routineLabels";

describe("routine delivery presentation", () => {
  it("keeps the completion explanation visible after one-time delivery", () => {
    expect(shouldShowRoutinePausedReason("delivered")).toBe(true);
  });

  it("lets actionable delivery recovery own the paused explanation", () => {
    expect(shouldShowRoutinePausedReason("retrying")).toBe(false);
    expect(shouldShowRoutinePausedReason("needs_review")).toBe(false);
  });
});
