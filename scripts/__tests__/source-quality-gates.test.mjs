import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { describe, expect, it } from "vitest";
import ts from "typescript";
import { measureSource } from "../source-quality/source-metrics.mjs";
import { inspectMeasurement } from "../check-source-quality.mjs";
import { analyzeProductionExports } from "../check-unused-exports.mjs";
import {
  rustCrateDependencies,
  typeScriptImportSpecifiers,
} from "../architecture/source-imports.mjs";

const root = path.resolve(import.meta.dirname, "../..");

function unbaselinedFailures(relativePath, measurement) {
  return inspectMeasurement(
    relativePath,
    measurement,
    new Map(),
    {},
    new Set(),
    new Set(),
  );
}

describe("source line ratchets", () => {
  it("fails on one line of growth beyond an explicit baseline", { timeout: 60_000 }, () => {
    const temporary = mkdtempSync(path.join(tmpdir(), "oomu-line-ratchet-"));
    try {
      const sourcePath = "src/app/components/ChatScreen.tsx";
      const actualLines = readFileSync(path.join(root, sourcePath), "utf8").split("\n").length - 1;
      const baseline = readFileSync(
        path.join(root, "scripts/source-line-baselines.tsv"),
        "utf8",
      ).replace(
        new RegExp(`^${sourcePath.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")}\\t\\d+`, "m"),
        `${sourcePath}\t${actualLines - 1}`,
      );
      const baselinePath = path.join(temporary, "source-lines.tsv");
      writeFileSync(baselinePath, baseline);

      const result = spawnSync("bash", ["scripts/check-rust-file-lines.sh"], {
        cwd: root,
        encoding: "utf8",
        env: {
          ...process.env,
          OOMU_SOURCE_LINE_BASELINE: baselinePath,
        },
      });

      expect(result.status).toBe(1);
      expect(result.stderr).toContain(`GROWTH ${sourcePath}`);
    } finally {
      rmSync(temporary, { force: true, recursive: true });
    }
  });

  it("rejects discretionary headroom in an existing baseline", { timeout: 60_000 }, () => {
    const temporary = mkdtempSync(path.join(tmpdir(), "oomu-line-headroom-"));
    try {
      const sourcePath = "src/app/components/ChatScreen.tsx";
      const actualLines = readFileSync(path.join(root, sourcePath), "utf8").split("\n").length - 1;
      const baseline = readFileSync(
        path.join(root, "scripts/source-line-baselines.tsv"),
        "utf8",
      ).replace(
        new RegExp(`^${sourcePath.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\t\\d+`, "m"),
        `${sourcePath}\t${actualLines + 1}`,
      );
      const baselinePath = path.join(temporary, "source-lines.tsv");
      writeFileSync(baselinePath, baseline);

      const result = spawnSync("bash", ["scripts/check-rust-file-lines.sh"], {
        cwd: root,
        encoding: "utf8",
        env: { ...process.env, OOMU_SOURCE_LINE_BASELINE: baselinePath },
      });

      expect(result.status).toBe(1);
      expect(result.stderr).toContain(`HEADROOM ${sourcePath}`);
    } finally {
      rmSync(temporary, { force: true, recursive: true });
    }
  });

  it("keeps the 1,500-line limit immutable", { timeout: 60_000 }, () => {
    const result = spawnSync("bash", ["scripts/check-rust-file-lines.sh"], {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env, OOMU_NEW_SOURCE_LINE_LIMIT: "999999" },
    });

    expect(result.status).toBe(1);
    expect(result.stderr).toContain("OOMU_NEW_SOURCE_LINE_LIMIT is not configurable");
  });
});

describe("source metric parsers", () => {
  it("measures human-authored tests in ordinary source test roots", () => {
    const measurement = measureSource(
      "src/app/__tests__/relocated.test.ts",
      "export const deterministicInput = 1;\n".repeat(1501),
    );
    expect(measurement.lines).toBe(1501);
  });

  it("detects compressed, byte-heavy, and extracted-but-still-giant source", () => {
    const compressed = measureSource(
      "src/app/compressed.ts",
      `export const compressed = "${"x".repeat(100_001)}";\n`,
    );
    const giant = measureSource(
      "src/app/extracted.ts",
      "export const extracted = true;\n".repeat(1501),
    );
    const oversizedScript = measureSource(
      "scripts/oversized-gate.mjs",
      "export const gate = true;\n".repeat(1501),
    );

    expect(compressed.bytes).toBeGreaterThan(100_000);
    expect(compressed.maxLineBytes).toBeGreaterThan(500);
    expect(giant.lines).toBe(1501);
    expect(oversizedScript.lines).toBe(1501);
    expect(unbaselinedFailures("src/app/compressed.ts", compressed)).toEqual(
      expect.arrayContaining([
        expect.stringContaining("NEW_EXCESS src/app/compressed.ts bytes="),
        expect.stringContaining("NEW_EXCESS src/app/compressed.ts maxLineBytes="),
      ]),
    );
    expect(unbaselinedFailures("src/app/extracted.ts", giant)).toContain(
      "NEW_OVERSIZED src/app/extracted.ts has 1501 lines (maximum 1500)",
    );
    expect(unbaselinedFailures("scripts/oversized-gate.mjs", oversizedScript)).toContain(
      "NEW_OVERSIZED scripts/oversized-gate.mjs has 1501 lines (maximum 1500)",
    );
  });

  it("measures parser-backed function length and branch complexity", () => {
    const longFunction = measureSource(
      "src/app/long-function.ts",
      ["export function longFunction() {", ...Array(120).fill("  void 0;"), "}", ""].join("\n"),
    );
    const complexFunction = measureSource(
      "src/app/complex-function.ts",
      [
        "export function complexFunction(values: boolean[]) {",
        ...Array.from({ length: 16 }, (_, index) => `  if (values[${index}]) return ${index};`),
        "  return -1;",
        "}",
        "",
      ].join("\n"),
    );

    expect(longFunction.functionLines).toBeGreaterThan(120);
    expect(complexFunction.complexity).toBeGreaterThan(15);
    expect(unbaselinedFailures("src/app/long-function.ts", longFunction)).toEqual(
      expect.arrayContaining([expect.stringContaining("NEW_EXCESS src/app/long-function.ts functionLines=")]),
    );
    expect(unbaselinedFailures("src/app/complex-function.ts", complexFunction)).toEqual(
      expect.arrayContaining([expect.stringContaining("NEW_EXCESS src/app/complex-function.ts complexity=")]),
    );
  });

  it("allows no growth or headroom in an exact metric exception", () => {
    const relativePath = "src/app/reviewed-complexity.ts";
    const baseline = {
      [relativePath]: { owner: "renderer/application", complexity: 16 },
    };
    const inspect = (complexity) => inspectMeasurement(
      relativePath,
      {
        kind: "typescript",
        lines: 20,
        bytes: 500,
        maxLineBytes: 80,
        functionLines: 20,
        functionLinesName: "reviewedComplexity",
        complexity,
        complexityName: "reviewedComplexity",
      },
      new Map(),
      baseline,
      new Set(),
      new Set(),
    );

    expect(inspect(16)).toEqual([]);
    expect(inspect(17)).toEqual([
      expect.stringContaining("GROWTH src/app/reviewed-complexity.ts complexity=17"),
    ]);
    expect(inspect(15)).toEqual([
      expect.stringContaining("HEADROOM src/app/reviewed-complexity.ts complexity=15"),
    ]);
  });
});

describe("source architecture and reachability ratchets", () => {
  it("fails when the reviewed cycle baseline is silently raised or lowered", { timeout: 60_000 }, () => {
    const temporary = mkdtempSync(path.join(tmpdir(), "oomu-cycle-ratchet-"));
    try {
      const baselinePath = path.join(temporary, "cycles.json");
      writeFileSync(baselinePath, JSON.stringify({ cycles: [] }));
      const result = spawnSync(process.execPath, ["scripts/check-module-cycles.mjs"], {
        cwd: root,
        encoding: "utf8",
        env: {
          ...process.env,
          OOMU_MODULE_CYCLE_BASELINE: baselinePath,
        },
      });

      expect(result.status).toBe(1);
      expect(result.stderr).toContain("NEW_CYCLE");
    } finally {
      rmSync(temporary, { force: true, recursive: true });
    }
  });

  it("parses real static and dynamic imports without comment or string false positives", () => {
    expect(typeScriptImportSpecifiers("fixture.ts", [
      "import value from './real';",
      "export { other } from './barrel';",
      "void import('./dynamic');",
      "const text = `import fake from './string'`;",
      "// import ignored from './comment'",
    ].join("\n"))).toEqual(["./real", "./barrel", "./dynamic"]);
    expect(rustCrateDependencies([
      "use crate::tools::Port;",
      "const TEXT: &str = \"crate::mcp::not_an_edge\";",
      "// crate::workflow_runtime::not_an_edge",
    ].join("\n"))).toEqual(["tools"]);
  });

  it("separates production, type-only, aliased, default, re-exported, and test-only exports", () => {
    const temporary = mkdtempSync(path.join(tmpdir(), "oomu-export-graph-"));
    try {
      const src = path.join(temporary, "src");
      const app = path.join(src, "app");
      mkdirSync(app, { recursive: true });
      writeFileSync(path.join(src, "library.ts"), [
        "export const used = 1;",
        "export default function defaultUsed() { return used; }",
        "export const testOnly = 2;",
        "export const dead = 3;",
        "export class OnlyType { value = ''; }",
        "",
      ].join("\n"));
      writeFileSync(path.join(src, "barrel.ts"), [
        "export { used as aliased, default } from './library';",
        "",
      ].join("\n"));
      writeFileSync(path.join(app, "page.ts"), [
        "import defaultUsed, { aliased } from '../barrel';",
        "import type { OnlyType } from '../library';",
        "const dynamicRegistry = { aliased, defaultUsed };",
        "const typed: OnlyType = { value: String(dynamicRegistry.aliased) };",
        "export default function Page() { return typed.value; }",
        "",
      ].join("\n"));
      writeFileSync(path.join(src, "library.test.ts"), [
        "import { testOnly } from './library';",
        "void testOnly;",
        "",
      ].join("\n"));
      const files = [
        path.join(src, "library.ts"),
        path.join(src, "barrel.ts"),
        path.join(app, "page.ts"),
        path.join(src, "library.test.ts"),
      ];
      const program = ts.createProgram(files, {
        module: ts.ModuleKind.ESNext,
        moduleResolution: ts.ModuleResolutionKind.Bundler,
        target: ts.ScriptTarget.ESNext,
      });
      const result = analyzeProductionExports(program, temporary);

      expect(result.unusedExports).toContain("src/library.ts#dead");
      expect(result.unusedExports).toContain("src/library.ts#OnlyType");
      expect(result.unusedExports).not.toContain("src/library.ts#testOnly");
      expect(result.testOnlyExports).toEqual(["src/library.ts#testOnly"]);
      expect(result.unusedExports).not.toContain("src/library.ts#used");
      expect(result.unusedExports).not.toContain("src/library.ts#default");
      expect(result.unusedExports).not.toContain("src/barrel.ts#aliased");
      expect(result.unusedExports).not.toContain("src/barrel.ts#default");
    } finally {
      rmSync(temporary, { force: true, recursive: true });
    }
  });
});
