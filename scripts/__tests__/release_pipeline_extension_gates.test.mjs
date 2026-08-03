import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import {
  discoverReleaseGateFiles,
  validateReleaseGateResult,
} from "../run-release-extension-gates.mjs";

describe("release extension gates", () => {
  it("discovers modules in deterministic lexical order", () => {
    const directory = join(mkdtempSync(join(tmpdir(), "oomu-release-extensions-")), "gates");
    mkdirSync(directory);
    writeFileSync(join(directory, "20-second.mjs"), "export const name='second';");
    writeFileSync(join(directory, "10-first.mjs"), "export const name='first';");
    expect(discoverReleaseGateFiles(directory)).toEqual(["10-first.mjs", "20-second.mjs"]);
  });

  it("accepts only explicit, serializable passing evidence", () => {
    expect(
      validateReleaseGateResult("measured_gate", { passed: true, evidence: { count: 3 } }),
    ).toEqual({ passed: true, evidence: { count: 3 } });
    expect(() =>
      validateReleaseGateResult("skipped_gate", { passed: false, evidence: { skipped: true } }),
    ).toThrow(/valid passing evidence/i);
    const recursive = { passed: true, evidence: {} };
    recursive.evidence.recursive = recursive;
    expect(() => validateReleaseGateResult("recursive_gate", recursive)).toThrow(
      /valid passing evidence/i,
    );
  });

  it("requires every canonical gate module to expose an executable run contract", async () => {
    const directory = resolve(import.meta.dirname, "..", "release-gates");
    const modules = discoverReleaseGateFiles(directory);
    const names = new Set();
    expect(modules.length).toBeGreaterThan(0);
    for (const fileName of modules) {
      const gate = await import(pathToFileURL(join(directory, fileName)).href);
      expect(gate.name, `${fileName} name export`).toMatch(/^[a-z][a-z0-9_]{2,63}$/);
      expect(names.has(gate.name), `${fileName} unique name export`).toBe(false);
      names.add(gate.name);
      expect(gate.run, `${fileName} run export`).toBeTypeOf("function");
    }
  });
});
