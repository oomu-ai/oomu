import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { repositoryPathFailure } from "../check-repository-hygiene.mjs";

function fixture(name) {
  return JSON.parse(
    readFileSync(path.join(import.meta.dirname, "fixtures", name), "utf8"),
  );
}

describe("repository hygiene tracked-output fixtures", () => {
  it("accepts clean source paths", () => {
    expect(
      fixture("repository-hygiene-clean.json").map(repositoryPathFailure),
    ).toEqual([null, null, null]);
  });

  it("rejects tracked build and cache output anywhere in the repository", () => {
    const failures = fixture("repository-hygiene-tracked-output.json")
      .map(repositoryPathFailure);
    expect(failures).toHaveLength(3);
    for (const failure of failures) {
      expect(failure).toContain("tracked build or cache output");
    }
  });
});
