import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join, relative, resolve, sep } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "..", "..");
const forbiddenMarkers = [
  "ai.eldris.mods.alignment",
  "ai.eldris.mods.developer_bundle",
  "Core Alignment Matrix",
  "Developer Bundle",
  "Developer Mod Bundle",
];
const textExtensions = new Set([
  ".css",
  ".html",
  ".js",
  ".json",
  ".mjs",
  ".py",
  ".rs",
  ".toml",
  ".ts",
  ".tsx",
  ".txt",
  ".yaml",
  ".yml",
]);

function productionFiles(relativeRoot, excludedSegments = new Set()) {
  const absoluteRoot = join(root, relativeRoot);
  const files = [];
  const visit = (path) => {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      if (excludedSegments.has(entry.name)) continue;
      const absolutePath = join(path, entry.name);
      if (entry.isDirectory()) {
        visit(absolutePath);
      } else if (entry.isFile() && textExtensions.has(extname(entry.name))) {
        files.push(absolutePath);
      }
    }
  };
  visit(absoluteRoot);
  return files;
}

function expectNoForbiddenMarkers(paths) {
  for (const path of paths) {
    const content = readFileSync(path, "utf8");
    for (const marker of forbiddenMarkers) {
      expect(content, `${relative(root, path)} must not ship ${marker}`).not.toContain(marker);
    }
  }
}

describe("release bundle Mod content", () => {
  it("excludes internal-only Mods from production code and declared resources", () => {
    const productionCode = [
      ...productionFiles("src", new Set(["__tests__"])).filter(
        (path) => !path.includes(`${sep}fixtures${sep}`) && !path.includes(".test."),
      ),
      ...productionFiles("src-tauri/src", new Set(["tests"])).filter(
        (path) => !path.endsWith("_test.rs") && !path.endsWith("_tests.rs"),
      ),
      ...productionFiles("src-tauri/resources"),
      join(root, "src-tauri", "tauri.conf.json"),
      join(root, "src-tauri", "tauri.release.conf.json"),
      join(root, "package.json"),
    ];

    for (const path of productionCode) {
      expect(statSync(path).isFile(), `${relative(root, path)} must be a file`).toBe(true);
    }
    expectNoForbiddenMarkers(productionCode);

    const tauri = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
    const declaredResources = tauri.bundle.resources.join("\n");
    expect(declaredResources).not.toMatch(/(?:^|\/)test_mods(?:\/|$)/u);
    expect(declaredResources).not.toMatch(/(?:^|\/)developer-tools(?:\/|$)/u);
  });

});
