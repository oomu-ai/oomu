import { describe, expect, it } from "vitest";
import { checkResultView, formatUpdateBytes, progressPercent } from "./applicationUpdates";

describe("application update presentation state", () => {
  it("maps native check results without inventing success", () => {
    expect(checkResultView({
      status: "update_available",
      origin: "manual",
      currentVersion: "0.1.2",
      availableVersion: "0.1.3",
      notes: "A careful update.",
      fullNotesAvailable: true,
    })).toMatchObject({
      status: "update_available",
      currentVersion: "0.1.2",
      availableVersion: "0.1.3",
    });
  });

  it("reports real bounded progress and readable byte counts", () => {
    expect(progressPercent(25, 100)).toBe(25);
    expect(progressPercent(110, 100)).toBe(100);
    expect(progressPercent(1, undefined)).toBeNull();
    expect(formatUpdateBytes(1_048_576)).toBe("1.0 MB");
  });
});
