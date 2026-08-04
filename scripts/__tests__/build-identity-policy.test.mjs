import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");

describe("build identity preparation order", () => {
  it("prepares exported frontend and portable Python before the batched Rust helpers", () => {
    const packageJson = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"));
    const phases = packageJson.scripts["tauri:prepare"].split(" && ");
    expect(phases.slice(0, 2)).toEqual([
      "npm run build",
      "node scripts/prepare-portable-python.mjs --release",
    ]);
    const reservation = phases.indexOf("node scripts/prepare-tauri-external-bins.mjs");
    const verification = phases.indexOf("node scripts/prepare-tauri-external-bins.mjs --verify");
    expect(reservation).toBe(2);
    expect(phases.slice(3, 8)).toEqual([
      "node scripts/prepare-rust-helpers.mjs --release",
      "node scripts/prepare-artifact-pdf-helper.mjs --release",
      "node scripts/prepare-vision-helper.mjs --release",
      "src-tauri/src-swift/build.sh",
      "node scripts/prepare-tauri-external-bins.mjs --verify",
    ]);
    for (const helper of [
      "node scripts/prepare-rust-helpers.mjs --release",
      "node scripts/prepare-artifact-pdf-helper.mjs --release",
      "node scripts/prepare-vision-helper.mjs --release",
    ]) {
      expect(phases.indexOf(helper)).toBeGreaterThan(1);
      expect(phases.indexOf(helper)).toBeGreaterThan(reservation);
      expect(phases.indexOf(helper)).toBeLessThan(verification);
    }
    for (const supersededHelper of [
      "node scripts/prepare-local-infer.mjs --release",
      "node scripts/prepare-pdf-helper.mjs --release",
      "node scripts/prepare-artifact-helper.mjs --release",
    ]) {
      expect(phases).not.toContain(supersededHelper);
    }
  });

  it("builds every Rust helper in one Cargo pass before copying any output", () => {
    const script = readFileSync(resolve(root, "scripts/prepare-rust-helpers.mjs"), "utf8");
    const cargoRun = script.indexOf('run("cargo", cargoArgs)');
    const firstCopy = script.indexOf("copyFileSync(helper.builtPath, helper.bundledPath)");
    const signing = script.indexOf('run("codesign", ["--force", "--sign", "-"');

    expect(script.match(/run\("cargo", cargoArgs\)/gu)).toHaveLength(1);
    expect(script).toContain('{ name: "local_infer", bundledStem: "local_infer" }');
    expect(script).toContain('{ name: "pdf_extract_helper", bundledStem: "pdf_extract_helper" }');
    expect(script).toContain('name: "artifact_build_helper"');
    expect(script).toContain('cargoArgs.push("--bin", helper.name)');
    expect(cargoRun).toBeGreaterThan(-1);
    expect(firstCopy).toBeGreaterThan(cargoRun);
    expect(signing).toBeGreaterThan(cargoRun);
  });
});
