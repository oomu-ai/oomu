import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");

describe("build identity preparation order", () => {
  it("prepares exported frontend and portable Python before cargo-based release helpers", () => {
    const packageJson = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"));
    const phases = packageJson.scripts["tauri:prepare"].split(" && ");
    expect(phases.slice(0, 2)).toEqual([
      "npm run build",
      "node scripts/prepare-portable-python.mjs --release",
    ]);
    const reservation = phases.indexOf("node scripts/prepare-tauri-external-bins.mjs");
    const verification = phases.indexOf("node scripts/prepare-tauri-external-bins.mjs --verify");
    expect(reservation).toBe(2);
    for (const helper of [
      "node scripts/prepare-local-infer.mjs --release",
      "node scripts/prepare-pdf-helper.mjs --release",
      "node scripts/prepare-artifact-helper.mjs --release",
      "node scripts/prepare-artifact-pdf-helper.mjs --release",
      "node scripts/prepare-vision-helper.mjs --release",
    ]) {
      expect(phases.indexOf(helper)).toBeGreaterThan(1);
      expect(phases.indexOf(helper)).toBeGreaterThan(reservation);
      expect(phases.indexOf(helper)).toBeLessThan(verification);
    }
  });
});
