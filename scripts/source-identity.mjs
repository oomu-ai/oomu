import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { lstatSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import process from "node:process";
import { createSanitizedChildEnvironment } from "./release-environment.mjs";

const SOURCE_REVISION_PATTERN = /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/u;

function gitBuffer(repositoryRoot, args) {
  const result = spawnSync("/usr/bin/git", args, {
    cwd: repositoryRoot,
    encoding: null,
    env: createSanitizedChildEnvironment({}, process.env),
    maxBuffer: 128 * 1024 * 1024,
  });
  if (result.status !== 0) {
    throw new Error(Buffer.concat([
      result.stderr ?? Buffer.alloc(0),
      result.stdout ?? Buffer.alloc(0),
    ]).toString("utf8").trim());
  }
  return result.stdout;
}

export function inspectSourceIdentity(repositoryRoot) {
  const sourceRevision = gitBuffer(repositoryRoot, ["rev-parse", "HEAD"])
    .toString("utf8")
    .trim();
  if (!SOURCE_REVISION_PATTERN.test(sourceRevision)) {
    throw new Error("The source revision is invalid.");
  }
  const status = gitBuffer(repositoryRoot, [
    "status", "--porcelain=v1", "-z", "--untracked-files=all",
  ]);
  const trackedDelta = gitBuffer(repositoryRoot, ["diff", "--binary", "HEAD", "--"]);
  const untracked = gitBuffer(repositoryRoot, [
    "ls-files", "-z", "--others", "--exclude-standard",
  ]).toString("utf8").split("\0").filter(Boolean).sort();
  const digest = createHash("sha256");
  digest.update("oomu-source-content-v1\0");
  digest.update(sourceRevision);
  digest.update("\0tracked-delta\0");
  digest.update(trackedDelta);
  for (const relativePath of untracked) {
    const path = resolve(repositoryRoot, relativePath);
    const metadata = lstatSync(path);
    digest.update("\0untracked\0");
    digest.update(relativePath);
    digest.update(`\0${metadata.mode & 0o7777}\0${metadata.size}\0`);
    if (metadata.isFile() && !metadata.isSymbolicLink()) {
      digest.update(readFileSync(path));
    } else {
      throw new Error("The source tree contains an unsupported untracked entry.");
    }
  }
  return {
    sourceRevision,
    sourceContentSha256: digest.digest("hex"),
    worktreeClean: status.length === 0,
    worktreeEntryCount: status.toString("utf8").split("\0").filter(Boolean).length,
    worktreeStatusSha256: createHash("sha256").update(status).digest("hex"),
  };
}
