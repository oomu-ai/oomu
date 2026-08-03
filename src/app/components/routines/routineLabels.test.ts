import { describe, expect, it } from "vitest";
import { backgroundStatusLabel } from "./routineLabels";

describe("backgroundStatusLabel", () => {
  it("uses calm localized guidance when this copy cannot register with macOS", () => {
    const t = (key: string) => ({
      "routines.background_signed_install":
        "This development copy can’t run scheduled work. Use a signed installed copy of OOMU.",
    })[key] ?? key;
    expect(backgroundStatusLabel(t, "requires_signed_install")).toBe(
      "This development copy can’t run scheduled work. Use a signed installed copy of OOMU.",
    );
  });
});
