import {
  createHash,
} from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, join, resolve, sep } from "node:path";
import { spawnSync } from "node:child_process";
import {
  DEVELOPMENT_BUNDLE_IDENTIFIER,
  PRODUCTION_BUNDLE_IDENTIFIER,
  localSigningIdentityPolicy,
} from "./local-signing-identity.mjs";

const root = resolve(import.meta.dirname, "..");
const signingIdentity = process.env.APPLE_SIGNING_IDENTITY?.trim() || "-";
const isAdHoc = signingIdentity === "-";
const defaultAppPath = join(
  root,
  "src-tauri",
  "target",
  "release",
  "bundle",
  "macos",
  isAdHoc ? "OOMU Development.app" : "OOMU.app",
);
const requestedAppPath = process.env.OOMU_LOCAL_APP_PATH?.trim();
const appPath = requestedAppPath ? resolve(root, requestedAppPath) : defaultAppPath;
const targetRoot = join(root, "src-tauri", "target");
if (!appPath.startsWith(`${targetRoot}${sep}`) || !appPath.endsWith(".app")) {
  throw new Error("OOMU_LOCAL_APP_PATH must name an app bundle inside src-tauri/target.");
}
const entitlements = join(root, "src-tauri", "entitlements.plist");
const expectedTeamId = process.env.APPLE_TEAM_ID?.trim() || "";

function run(command, args, label) {
  const result = spawnSync(command, args, { encoding: "utf8" });
  if (result.status !== 0) {
    const detail = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(`${label} failed${detail ? `:\n${detail}` : "."}`);
  }
  return result;
}

function walk(path) {
  const entries = [];
  for (const entry of readdirSync(path, { withFileTypes: true })) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) {
      entries.push(...walk(child));
    } else if (entry.isFile()) {
      entries.push(child);
    }
  }
  return entries;
}

if (process.platform !== "darwin") {
  throw new Error("Local macOS app signing is only available on macOS.");
}
if (!existsSync(appPath) || !statSync(appPath).isDirectory()) {
  throw new Error(`OOMU.app was not found at ${appPath}`);
}
if (!existsSync(entitlements)) {
  throw new Error(`Reviewed app entitlements were not found at ${entitlements}`);
}
if (!isAdHoc && !expectedTeamId) {
  throw new Error("APPLE_TEAM_ID is required for Developer ID signing.");
}

const infoPlist = join(appPath, "Contents", "Info.plist");
if (!existsSync(infoPlist)) {
  throw new Error(`The app Info.plist was not found at ${infoPlist}`);
}
const bundleIdentifier = run(
  "/usr/bin/plutil",
  ["-extract", "CFBundleIdentifier", "raw", "-o", "-", "--", infoPlist],
  "Reading the app bundle identifier",
).stdout.trim();
const signingPolicy = localSigningIdentityPolicy({
  appPath,
  bundleIdentifier,
  signingIdentity,
  expectedTeamId,
});
if (
  signingPolicy.expectedBundleIdentifier !== (isAdHoc
    ? DEVELOPMENT_BUNDLE_IDENTIFIER
    : PRODUCTION_BUNDLE_IDENTIFIER)
) {
  throw new Error("The selected signing identity does not match the reviewed app channel.");
}

function signingArguments(path, selectedEntitlements = null) {
  const args = ["--force", isAdHoc ? "--timestamp=none" : "--timestamp"];
  args.push("--options", "runtime", "--sign", signingIdentity);
  if (selectedEntitlements) args.push("--entitlements", selectedEntitlements);
  args.push(path);
  return args;
}

const nestedMachO = walk(appPath)
  .filter((path) => {
    const probe = spawnSync("/usr/bin/file", ["-b", path], { encoding: "utf8" });
    return probe.status === 0 && /Mach-O/u.test(probe.stdout);
  })
  .sort((left, right) => right.split(sep).length - left.split(sep).length);

for (const path of nestedMachO) {
  run(
    "/usr/bin/codesign",
    signingArguments(path),
    `Signing ${basename(path)}`,
  );
}

const helperIntegrityPath = join(
  appPath,
  "Contents",
  "Resources",
  "oomu-helper-integrity.json",
);
const helperNames = [
  "artifact_build_helper",
  "oomu-artifact-pdf-helper",
];
const helpers = Object.fromEntries(
  helperNames.map((name) => {
    const path = join(appPath, "Contents", "MacOS", name);
    if (!existsSync(path) || !statSync(path).isFile()) {
      throw new Error(`Required artifact helper is missing: ${name}`);
    }
    return [
      name,
      createHash("sha256").update(readFileSync(path)).digest("hex"),
    ];
  }),
);
mkdirSync(join(appPath, "Contents", "Resources"), { recursive: true });
writeFileSync(
  helperIntegrityPath,
  `${JSON.stringify({ schemaVersion: 1, helpers }, null, 2)}\n`,
  { mode: 0o644 },
);

run(
  "/usr/bin/codesign",
  [
    ...signingArguments(appPath, entitlements).slice(0, -1),
    "--identifier",
    bundleIdentifier,
    appPath,
  ],
  "Signing OOMU.app",
);
run(
  "/usr/bin/codesign",
  ["--verify", "--deep", "--strict", "--verbose=4", appPath],
  "Verifying OOMU.app",
);

const details = run(
  "/usr/bin/codesign",
  ["-d", "--verbose=4", appPath],
  "Reading OOMU.app signing identity",
).stderr;
if (!details.includes(`Identifier=${bundleIdentifier}`) || !details.includes("Info.plist entries=")) {
  throw new Error("OOMU.app did not retain its bundle identifier and bound Info.plist after signing.");
}
if (!/flags=0x[0-9a-f]+\([^)]*\bruntime\b[^)]*\)/iu.test(details)) {
  throw new Error("OOMU.app was not signed with the hardened runtime.");
}
if (!isAdHoc && !details.includes(`TeamIdentifier=${expectedTeamId}`)) {
  throw new Error(`OOMU.app was not signed for Apple team ${expectedTeamId}.`);
}
console.log(
  `[local-sign] Verified ${appPath} (${isAdHoc ? "ad hoc" : signingIdentity})`,
);
