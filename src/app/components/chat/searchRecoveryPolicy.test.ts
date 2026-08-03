import { describe, expect, it } from "vitest";
import { runWeb } from "./searchRecoveryPolicy";

describe("interrupted search recovery", () => {
  it("rebuilds public evidence only for ordinary and interrupted turns", () => {
    expect(runWeb(false)).toBe(true);
    expect(runWeb(true, { turnState: "interrupted" })).toBe(true);
    expect(runWeb(true, { turnState: "accepted" })).toBe(false);
    expect(runWeb(true)).toBe(false);
  });
});
