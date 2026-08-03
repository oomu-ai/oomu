import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  lstatSync,
  readFileSync,
  readdirSync,
  realpathSync,
} from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import {
  artifactDigestForEntries,
  collectTreeEntries,
} from "./release-manifest.mjs";

function walk(directory) {
  const paths = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isSymbolicLink()) continue;
    paths.push(path);
    if (entry.isDirectory()) paths.push(...walk(path));
  }
  return paths;
}

function targetDepth(path) {
  return path.split(sep).length;
}

export function orderedNestedCodeTargets(
  appPath,
  fileTool = "/usr/bin/file",
  classify = null,
) {
  const application = realpathSync(resolve(appPath));
  const paths = walk(application);
  const containers = paths.filter((path) =>
    lstatSync(path).isDirectory() && /\.(?:app|appex|framework|xpc)$/u.test(path));
  const machO = paths.filter((path) => {
    if (!lstatSync(path).isFile()) return false;
    if (classify) return /Mach-O/u.test(classify(path));
    const probe = spawnSync(fileTool, ["-b", path], { encoding: "utf8" });
    if (probe.error) throw probe.error;
    return probe.status === 0 && /Mach-O/u.test(probe.stdout);
  });
  return [...new Set([...containers, ...machO])].sort((left, right) => {
    const depth = targetDepth(right) - targetDepth(left);
    return depth || left.localeCompare(right);
  });
}

export function codeSigningOrder(appPath, fileTool = "/usr/bin/file", classify = null) {
  const application = realpathSync(resolve(appPath));
  return [
    ...orderedNestedCodeTargets(application, fileTool, classify),
    application,
  ];
}

export function signedArtifactIdentity(path) {
  const target = realpathSync(resolve(path));
  if (lstatSync(target).isDirectory()) {
    return artifactDigestForEntries(collectTreeEntries(target));
  }
  return `sha256:${createHash("sha256").update(readFileSync(target)).digest("hex")}`;
}

export function assertSignedArtifactUnchanged(path, expectedIdentity, boundary) {
  const actualIdentity = signedArtifactIdentity(path);
  if (actualIdentity !== expectedIdentity) {
    throw new Error(
      `release_post_sign_mutation:${boundary}:${relative(resolve(path, ".."), resolve(path)) || "artifact"}`,
    );
  }
  return actualIdentity;
}
