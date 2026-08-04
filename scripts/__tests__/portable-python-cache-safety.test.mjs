import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");
const preparationSource = readFileSync(
  resolve(root, "scripts/prepare-portable-python.mjs"),
  "utf8",
);

describe("portable Python cache safety", () => {
  it("disables bytecode writes for every preparation subprocess", () => {
    const runHelper = preparationSource.match(
      /function run\(command, args\) \{[\s\S]*?\n\}/,
    )?.[0];

    expect(runHelper).toContain('PYTHONDONTWRITEBYTECODE: "1"');
  });

  it("prunes caches after the final interpreter validation", () => {
    const validationIndex = preparationSource.lastIndexOf(
      'run(pythonBinary, ["--version"]);',
    );
    const finalPruneIndex = preparationSource.lastIndexOf(
      "prunePythonCaches(resourcePythonDir);",
    );
    const finalManifestIndex = preparationSource.lastIndexOf(
      "writeNativeFileManifest(resourcePythonDir);",
    );

    expect(validationIndex).toBeGreaterThan(0);
    expect(finalPruneIndex).toBeGreaterThan(validationIndex);
    expect(finalManifestIndex).toBeGreaterThan(finalPruneIndex);
  });
});
