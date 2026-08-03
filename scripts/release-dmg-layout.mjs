import { lstatSync, readlinkSync, symlinkSync } from "node:fs";
import { basename, join } from "node:path";

const APPLICATIONS_DIRECTORY = "/Applications";

export function stageApplicationsShortcut(dmgRoot) {
  const shortcutPath = join(dmgRoot, "Applications");
  symlinkSync(APPLICATIONS_DIRECTORY, shortcutPath, "dir");
  return shortcutPath;
}

export function assertDragInstallDmgRoot(dmgRoot, appPath) {
  const appName = basename(appPath);
  if (!appName.endsWith(".app") || appName === ".app") {
    throw new Error("Release DMG application name is invalid.");
  }
  const stagedApp = join(dmgRoot, appName);
  if (!lstatSync(stagedApp).isDirectory()) {
    throw new Error("Release DMG does not contain the application bundle.");
  }
  const shortcutPath = join(dmgRoot, "Applications");
  if (!lstatSync(shortcutPath).isSymbolicLink()) {
    throw new Error("Release DMG Applications item is not a shortcut.");
  }
  if (readlinkSync(shortcutPath) !== APPLICATIONS_DIRECTORY) {
    throw new Error("Release DMG Applications shortcut has the wrong destination.");
  }
  return Object.freeze({ stagedApp, shortcutPath });
}
