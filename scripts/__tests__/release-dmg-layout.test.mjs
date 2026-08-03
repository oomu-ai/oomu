import {
  mkdirSync,
  mkdtempSync,
  readlinkSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  assertDragInstallDmgRoot,
  stageApplicationsShortcut,
} from "../release-dmg-layout.mjs";

const temporaryRoots = [];

afterEach(() => {
  for (const path of temporaryRoots.splice(0)) {
    rmSync(path, { recursive: true, force: true });
  }
});

describe("macOS drag-install DMG layout", () => {
  it("stages OOMU beside an Applications shortcut", () => {
    const dmgRoot = mkdtempSync(join(tmpdir(), "oomu-dmg-layout-"));
    temporaryRoots.push(dmgRoot);
    const appPath = join(dmgRoot, "OOMU.app");
    mkdirSync(appPath);

    const shortcutPath = stageApplicationsShortcut(dmgRoot);
    const layout = assertDragInstallDmgRoot(dmgRoot, appPath);

    expect(layout.stagedApp).toBe(appPath);
    expect(layout.shortcutPath).toBe(shortcutPath);
    expect(readlinkSync(shortcutPath)).toBe("/Applications");
  });

  it("rejects a DMG root without its Applications shortcut", () => {
    const dmgRoot = mkdtempSync(join(tmpdir(), "oomu-dmg-layout-"));
    temporaryRoots.push(dmgRoot);
    const appPath = join(dmgRoot, "OOMU.app");
    mkdirSync(appPath);

    expect(() => assertDragInstallDmgRoot(dmgRoot, appPath)).toThrow();
  });
});
