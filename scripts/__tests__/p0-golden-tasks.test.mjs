import { describe, expect, it } from "vitest";
import { discoverGoldenTasks, discoveryReport } from "../p0-golden-tasks.mjs";

describe("P0 golden task discovery", () => {
  it("maps exactly ten postcondition-based tasks to registered production commands", () => {
    const { definitions, failures } = discoverGoldenTasks();
    expect(failures).toEqual([]);
    expect(definitions.tasks).toHaveLength(10);
  });

  it("records current build and machine identity without claiming model execution", () => {
    const report = discoveryReport();
    expect(report.status).toBe("passed");
    expect(report.build.sourceRevision).toMatch(/^[0-9a-f]{40}$/);
    expect(report.machine.architecture).toBeTruthy();
    expect(report.model.executionStatus).toBe("not-run");
  });
});
