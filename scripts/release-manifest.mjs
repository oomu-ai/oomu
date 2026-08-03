#!/usr/bin/env node

import {
  chmodSync,
  closeSync,
  existsSync,
  fsyncSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  realpathSync,
  renameSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import {
  createHash,
  createPrivateKey,
  createPublicKey,
  sign as signPayload,
  verify as verifySignature,
} from "node:crypto";
import { dirname, isAbsolute, join, posix, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

export const MANIFEST_KIND = "oomu.release-tree-manifest";
export const MANIFEST_SCHEMA_VERSION = 3;
// Reviewed Architect Root release key. Rotating it requires a source review and
// coordinated secure provisioning of the corresponding private key in CI.
export const TRUSTED_RELEASE_PUBLIC_KEY_HEX =
  "d40713a67f6ec73f2cadfa89bbc92d4535055655d368cc0606051b6b60f29620";

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function stableJson(value) {
  if (Array.isArray(value)) {
    return `[${value.map((entry) => stableJson(entry)).join(",")}]`;
  }
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function isInside(root, candidate) {
  const between = relative(resolve(root), resolve(candidate));
  return between === "" || (!between.startsWith("..") && !between.startsWith(sep));
}

function normalizedRelativePath(value) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.includes("\\") ||
    isAbsolute(value) ||
    value.startsWith("/") ||
    value.split("/").some((part) => part === "" || part === "." || part === "..") ||
    posix.normalize(value) !== value
  ) {
    throw new Error(`Manifest entry path is not a normalized relative path: ${String(value)}`);
  }
  return value;
}

function entryForFile(root, absolutePath, relativePath) {
  const stats = lstatSync(absolutePath);
  if (stats.isSymbolicLink()) {
    const target = readlinkSync(absolutePath);
    const resolvedTarget = realpathSync(absolutePath);
    if (!isInside(root, resolvedTarget)) {
      throw new Error(`Symbolic link escapes the release tree: ${relativePath} -> ${target}`);
    }
    const bytes = Buffer.from(target, "utf8");
    return {
      path: normalizedRelativePath(relativePath),
      type: "symlink",
      size_bytes: bytes.byteLength,
      sha256: sha256(bytes),
      link_target: target,
    };
  }
  if (!stats.isFile()) {
    throw new Error(`Unsupported release-tree entry type: ${relativePath}`);
  }
  const bytes = readFileSync(absolutePath);
  return {
    path: normalizedRelativePath(relativePath),
    type: "file",
    size_bytes: stats.size,
    sha256: sha256(bytes),
  };
}

export function collectTreeEntries(treeRoot) {
  const root = realpathSync(resolve(treeRoot));
  if (!lstatSync(root).isDirectory()) {
    throw new Error(`Release tree is not a directory: ${root}`);
  }
  const entries = [];

  function walk(directory, prefix = "") {
    for (const child of readdirSync(directory, { withFileTypes: true }).sort((a, b) =>
      a.name < b.name ? -1 : a.name > b.name ? 1 : 0,
    )) {
      const relativePath = prefix ? `${prefix}/${child.name}` : child.name;
      normalizedRelativePath(relativePath);
      const absolutePath = join(directory, child.name);
      if (child.isSymbolicLink()) {
        entries.push(entryForFile(root, absolutePath, relativePath));
      } else if (child.isDirectory()) {
        walk(absolutePath, relativePath);
      } else if (child.isFile()) {
        entries.push(entryForFile(root, absolutePath, relativePath));
      } else {
        throw new Error(`Unsupported release-tree entry type: ${relativePath}`);
      }
    }
  }

  walk(root);
  entries.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  if (entries.length === 0) {
    throw new Error("Release tree is empty.");
  }
  return entries;
}

export function artifactDigestForEntries(entries) {
  return `sha256:${sha256(stableJson(entries))}`;
}

function topLevelPath(entryPath) {
  return entryPath.split("/", 1)[0];
}

function entriesForSubtree(entries, prefix) {
  const normalizedPrefix = normalizedRelativePath(prefix);
  const nestedPrefix = `${normalizedPrefix}/`;
  return entries
    .filter((entry) => entry.path === normalizedPrefix || entry.path.startsWith(nestedPrefix))
    .map((entry) => ({
      ...entry,
      path: entry.path === normalizedPrefix
        ? normalizedPrefix.split("/").at(-1)
        : entry.path.slice(nestedPrefix.length),
    }));
}

export function subtreeDigestsForEntries(entries) {
  return [...new Set(entries.map((entry) => topLevelPath(entry.path)))]
    .sort()
    .map((pathPrefix) => {
      const subtreeEntries = entriesForSubtree(entries, pathPrefix);
      return {
        path_prefix: pathPrefix,
        entry_count: subtreeEntries.length,
        artifact_digest: artifactDigestForEntries(subtreeEntries),
      };
    });
}

export function atomicWriteFile(filePath, contents, mode = 0o444) {
  const destination = resolve(filePath);
  mkdirSync(dirname(destination), { recursive: true });
  const temporary = `${destination}.tmp-${process.pid}-${Date.now()}`;
  let descriptor;
  try {
    descriptor = openSync(temporary, "wx", 0o600);
    writeFileSync(descriptor, contents);
    fsyncSync(descriptor);
    closeSync(descriptor);
    descriptor = undefined;
    chmodSync(temporary, mode);
    renameSync(temporary, destination);
  } catch (error) {
    if (descriptor !== undefined) {
      closeSync(descriptor);
    }
    if (existsSync(temporary)) {
      unlinkSync(temporary);
    }
    throw error;
  }
}

function manifestPayload({
  buildIdentifier,
  sourceRevision,
  artifactIdentifier,
  treeLabel,
  generatedAt,
  entries,
  releaseProvenance,
}) {
  if (!buildIdentifier?.trim()) throw new Error("A build identifier is required.");
  if (!/^[0-9a-f]{40}$/i.test(sourceRevision ?? "")) {
    throw new Error("Source revision must be a full 40-character Git commit hash.");
  }
  if (!artifactIdentifier?.trim()) throw new Error("An artifact identifier is required.");
  if (releaseProvenance !== undefined) {
    validateReleaseProvenance(releaseProvenance, sourceRevision, entries);
  }
  return {
    schema_version: MANIFEST_SCHEMA_VERSION,
    kind: MANIFEST_KIND,
    generated_at: generatedAt ?? new Date().toISOString(),
    source_revision: sourceRevision,
    build_identifier: buildIdentifier,
    artifact_identifier: artifactIdentifier,
    artifact_digest: artifactDigestForEntries(entries),
    subtrees: subtreeDigestsForEntries(entries),
    tree_label: treeLabel,
    entry_count: entries.length,
    entries,
    ...(releaseProvenance === undefined ? {} : { release_provenance: releaseProvenance }),
  };
}

function validateReleaseProvenance(provenance, sourceRevision, entries) {
  const actionCommits = Object.values(provenance?.actionCommitShas ?? {});
  const executables = Object.values(provenance?.executableEvidence ?? {});
  const candidate = provenance?.releaseCandidateIntegrity;
  const applicationSubtrees = subtreeDigestsForEntries(entries)
    .filter((subtree) => subtree.path_prefix.endsWith(".app"));
  const candidateIdentityValid =
    candidate?.kind === "oomu.release-candidate-integrity" &&
    candidate?.schemaVersion === 1 &&
    /^[0-9a-f]{64}$/u.test(candidate?.reportSha256 ?? "") &&
    applicationSubtrees.length === 1 &&
    candidate?.applicationTreeDigest === applicationSubtrees[0].artifact_digest &&
    typeof candidate?.bundleIdentifier === "string" &&
    candidate.bundleIdentifier.length > 0 &&
    /^[A-Z0-9]{10}$/u.test(candidate?.teamId ?? "") &&
    candidate?.authority?.startsWith("Developer ID Application:") &&
    /^\d+$/u.test(String(candidate?.buildNumber ?? "")) &&
    /^[0-9a-f]{40,64}$/u.test(candidate?.codeDirectoryHash ?? "") &&
    /^[0-9a-f]{64}$/u.test(candidate?.designatedRequirementSha256 ?? "") &&
    candidate?.hardenedRuntime === true &&
    /^[0-9a-f]{64}$/u.test(candidate?.entitlementDigest ?? "") &&
    candidate?.gatekeeperAccepted === true &&
    candidate?.notarizationAccepted === true &&
    Number.isSafeInteger(candidate?.codeObjectCount) &&
    candidate.codeObjectCount > 0;
  const baseValid =
    provenance?.kind === "oomu.release-provenance" &&
    provenance?.workflowSourceCommit === sourceRevision &&
    typeof provenance?.releasePolicyId === "string" &&
    provenance.releasePolicyId.length > 0 &&
    /^[0-9a-f]{64}$/u.test(provenance?.releasePolicyDigest ?? "") &&
    typeof provenance?.runnerIdentity?.architecture === "string" &&
    actionCommits.length > 0 &&
    actionCommits.every((sha) => /^[0-9a-f]{40}$/u.test(sha)) &&
    executables.length > 0 &&
    executables.every((tool) =>
      tool?.executable?.startsWith("/") && /^[0-9a-f]{64}$/u.test(tool?.sha256 ?? "")) &&
    /^\d+\.\d+\.\d+$/u.test(provenance?.rustToolchain?.channel ?? "") &&
    typeof provenance?.rustToolchain?.target === "string" &&
    provenance?.xcodeSdk?.developerDirectory?.startsWith("/") &&
    typeof provenance?.xcodeSdk?.sdkVersion === "string" &&
    typeof provenance?.buildSignPhaseIsolated === "boolean" &&
    provenance?.signedOutputDigest === artifactDigestForEntries(entries);
  const isolatedValid = provenance?.buildSignPhaseIsolated !== true || (
    typeof provenance?.unsignedHandoffIdentity === "string" &&
    /^sha256:[0-9a-f]{64}$/u.test(provenance?.unsignedTreeDigest ?? "") &&
    /^[0-9a-f]{64}$/u.test(provenance?.unsignedPayloadDigest ?? "") &&
    /^[0-9a-f]{64}$/u.test(provenance?.unsignedArchiveDigest ?? "") &&
    provenance?.unsignedBuilderToolchain &&
    provenance?.protectedSignerToolchain
  );
  if (!baseValid || !isolatedValid || !candidateIdentityValid) {
    throw new Error("Release provenance is incomplete, malformed, or bound to different bytes.");
  }
}

export function generateReleaseManifest({
  treeRoot,
  manifestPath,
  buildIdentifier,
  sourceRevision,
  artifactIdentifier,
  privateKeyPath,
  generatedAt,
  trustedPublicKeyHex = TRUSTED_RELEASE_PUBLIC_KEY_HEX,
  releaseProvenance,
}) {
  const tree = realpathSync(resolve(treeRoot));
  const destination = resolve(manifestPath);
  if (isInside(tree, destination)) {
    throw new Error("The manifest must be outside the tree it authenticates.");
  }
  const entries = collectTreeEntries(tree);
  const payload = manifestPayload({
    buildIdentifier,
    sourceRevision,
    artifactIdentifier,
    treeLabel: tree.split(sep).at(-1),
    generatedAt,
    entries,
    releaseProvenance,
  });
  const privateKey = createPrivateKey(readFileSync(resolve(privateKeyPath)));
  if (privateKey.asymmetricKeyType !== "ed25519") {
    throw new Error("Release manifests must be signed with an Ed25519 private key.");
  }
  const payloadBytes = Buffer.from(stableJson(payload), "utf8");
  const publicKey = createPublicKey(privateKey);
  const rawPublicKey = Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url");
  if (rawPublicKey.toString("hex") !== trustedPublicKeyHex) {
    throw new Error("Manifest private key does not match the reviewed release trust root.");
  }
  const manifest = {
    ...payload,
    payload_sha256: sha256(payloadBytes),
    signature: {
      algorithm: "ed25519",
      public_key_hex: rawPublicKey.toString("hex"),
      key_fingerprint_sha256: sha256(
        publicKey.export({ type: "spki", format: "der" }),
      ),
      value_base64: signPayload(null, payloadBytes, privateKey).toString("base64"),
    },
  };
  atomicWriteFile(destination, `${JSON.stringify(manifest, null, 2)}\n`);
  return manifest;
}

function validateDeclaredEntries(entries, declaredCount) {
  if (!Array.isArray(entries)) throw new Error("Manifest entries must be an array.");
  if (!Number.isSafeInteger(declaredCount) || declaredCount !== entries.length) {
    throw new Error(
      `Manifest entry count mismatch: declared ${declaredCount}, found ${entries.length}.`,
    );
  }
  const seen = new Set();
  for (const entry of entries) {
    normalizedRelativePath(entry?.path);
    if (seen.has(entry.path)) throw new Error(`Duplicate manifest path: ${entry.path}`);
    seen.add(entry.path);
    if (!Number.isSafeInteger(entry.size_bytes) || entry.size_bytes < 0) {
      throw new Error(`Invalid byte size for ${entry.path}.`);
    }
    if (!/^[0-9a-f]{64}$/.test(entry.sha256 ?? "")) {
      throw new Error(`Invalid SHA-256 digest for ${entry.path}.`);
    }
    if (entry.type !== "file" && entry.type !== "symlink") {
      throw new Error(`Invalid entry type for ${entry.path}.`);
    }
    if (entry.type === "symlink" && typeof entry.link_target !== "string") {
      throw new Error(`Missing symbolic-link target for ${entry.path}.`);
    }
  }
}

function validateDeclaredSubtrees(subtrees, entries) {
  const expected = subtreeDigestsForEntries(entries);
  if (!Array.isArray(subtrees) || stableJson(subtrees) !== stableJson(expected)) {
    throw new Error("Manifest subtree digests do not match its declared entries.");
  }
}

function unsignedPayload(manifest) {
  const payload = { ...manifest };
  delete payload.payload_sha256;
  delete payload.signature;
  return payload;
}

function verifyExactTreeEntries(declaredEntries, actualEntries) {
  const declaredByPath = new Map(declaredEntries.map((entry) => [entry.path, entry]));
  const actualByPath = new Map(actualEntries.map((entry) => [entry.path, entry]));
  for (const path of declaredByPath.keys()) {
    if (!actualByPath.has(path)) throw new Error(`Declared release entry is missing: ${path}`);
  }
  for (const path of actualByPath.keys()) {
    if (!declaredByPath.has(path)) throw new Error(`Undeclared release entry exists: ${path}`);
  }
  for (const actual of actualEntries) {
    const declared = declaredByPath.get(actual.path);
    if (declared.type !== actual.type) throw new Error(`Entry type mismatch: ${actual.path}`);
    if (declared.size_bytes !== actual.size_bytes) throw new Error(`Size mismatch: ${actual.path}`);
    if (declared.sha256 !== actual.sha256) throw new Error(`Hash mismatch: ${actual.path}`);
    if (declared.link_target !== actual.link_target) {
      throw new Error(`Symbolic-link target mismatch: ${actual.path}`);
    }
  }
}

export function verifyReleaseManifest({
  treeRoot,
  manifestPath,
  publicKeyPath,
  expectedBuildIdentifier,
  expectedSourceRevision,
  expectedArtifactIdentifier,
  expectedReleaseProvenance,
  trustedPublicKeyHex = TRUSTED_RELEASE_PUBLIC_KEY_HEX,
}) {
  const tree = realpathSync(resolve(treeRoot));
  const manifest = JSON.parse(readFileSync(resolve(manifestPath), "utf8"));
  if (manifest.schema_version !== MANIFEST_SCHEMA_VERSION || manifest.kind !== MANIFEST_KIND) {
    throw new Error("Unsupported release manifest schema.");
  }
  validateDeclaredEntries(manifest.entries, manifest.entry_count);
  validateDeclaredSubtrees(manifest.subtrees, manifest.entries);
  if (expectedBuildIdentifier && manifest.build_identifier !== expectedBuildIdentifier) {
    throw new Error("Manifest build identifier does not match the current build.");
  }
  if (expectedSourceRevision && manifest.source_revision !== expectedSourceRevision) {
    throw new Error("Manifest source revision does not match the audited revision.");
  }
  if (expectedArtifactIdentifier && manifest.artifact_identifier !== expectedArtifactIdentifier) {
    throw new Error("Manifest artifact identifier does not match the current artifact.");
  }
  if (!/^[0-9a-f]{40}$/i.test(manifest.source_revision ?? "")) {
    throw new Error("Manifest source revision is malformed.");
  }
  if (
    expectedReleaseProvenance !== undefined &&
    stableJson(manifest.release_provenance) !== stableJson(expectedReleaseProvenance)
  ) {
    throw new Error("Manifest release provenance does not match the protected release evidence.");
  }

  const actualEntries = collectTreeEntries(tree);
  verifyExactTreeEntries(manifest.entries, actualEntries);

  const expectedArtifactDigest = artifactDigestForEntries(actualEntries);
  if (manifest.artifact_digest !== expectedArtifactDigest) {
    throw new Error("Manifest artifact digest does not match the exact release tree.");
  }
  verifyManifestSignature({ manifest, publicKeyPath, trustedPublicKeyHex });
  return manifest;
}

function verifyManifestSignature({
  manifest,
  publicKeyPath,
  trustedPublicKeyHex,
}) {
  const payload = unsignedPayload(manifest);
  const payloadBytes = Buffer.from(stableJson(payload), "utf8");
  if (manifest.payload_sha256 !== sha256(payloadBytes)) {
    throw new Error("Manifest payload hash mismatch.");
  }
  if (manifest.signature?.algorithm !== "ed25519") {
    throw new Error("Manifest signature algorithm is not Ed25519.");
  }
  const publicKey = createPublicKey(readFileSync(resolve(publicKeyPath)));
  if (publicKey.asymmetricKeyType !== "ed25519") {
    throw new Error("Release manifest trust key must be Ed25519.");
  }
  const rawPublicKey = Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url");
  if (
    rawPublicKey.toString("hex") !== trustedPublicKeyHex ||
    manifest.signature.public_key_hex !== trustedPublicKeyHex
  ) {
    throw new Error("Manifest signature does not use the reviewed release trust root.");
  }
  const fingerprint = sha256(publicKey.export({ type: "spki", format: "der" }));
  if (manifest.signature.key_fingerprint_sha256 !== fingerprint) {
    throw new Error("Manifest signing key does not match the trusted release key.");
  }
  const signature = Buffer.from(manifest.signature.value_base64 ?? "", "base64");
  if (!verifySignature(null, payloadBytes, publicKey, signature)) {
    throw new Error("Manifest signature verification failed.");
  }
}

export function verifyReleaseManifestSubtree({
  treeRoot,
  manifestPath,
  publicKeyPath,
  pathPrefix,
  expectedBuildIdentifier,
  expectedSourceRevision,
  expectedArtifactIdentifier,
  trustedPublicKeyHex = TRUSTED_RELEASE_PUBLIC_KEY_HEX,
}) {
  const tree = realpathSync(resolve(treeRoot));
  const manifest = JSON.parse(readFileSync(resolve(manifestPath), "utf8"));
  if (manifest.schema_version !== MANIFEST_SCHEMA_VERSION || manifest.kind !== MANIFEST_KIND) {
    throw new Error("Unsupported release manifest schema.");
  }
  validateDeclaredEntries(manifest.entries, manifest.entry_count);
  validateDeclaredSubtrees(manifest.subtrees, manifest.entries);
  if (expectedBuildIdentifier && manifest.build_identifier !== expectedBuildIdentifier) {
    throw new Error("Manifest build identifier does not match the current build.");
  }
  if (expectedSourceRevision && manifest.source_revision !== expectedSourceRevision) {
    throw new Error("Manifest source revision does not match the audited revision.");
  }
  if (expectedArtifactIdentifier && manifest.artifact_identifier !== expectedArtifactIdentifier) {
    throw new Error("Manifest artifact identifier does not match the current artifact.");
  }
  verifyManifestSignature({ manifest, publicKeyPath, trustedPublicKeyHex });

  const prefix = normalizedRelativePath(pathPrefix);
  const declaredEntries = entriesForSubtree(manifest.entries, prefix);
  if (declaredEntries.length === 0) {
    throw new Error(`Manifest does not contain subtree: ${prefix}`);
  }
  const actualEntries = collectTreeEntries(tree);
  verifyExactTreeEntries(declaredEntries, actualEntries);
  const declaredSubtree = manifest.subtrees.find((entry) => entry.path_prefix === prefix);
  if (
    !declaredSubtree ||
    declaredSubtree.entry_count !== actualEntries.length ||
    declaredSubtree.artifact_digest !== artifactDigestForEntries(actualEntries)
  ) {
    throw new Error(`Installed subtree digest mismatch: ${prefix}`);
  }
  return { manifest, subtree: declaredSubtree };
}

function parseArgs(argv) {
  const command = argv[0];
  const values = {};
  for (let index = 1; index < argv.length; index += 1) {
    const key = argv[index];
    if (!key.startsWith("--")) throw new Error(`Unexpected argument: ${key}`);
    const value = argv[index + 1];
    if (!value || value.startsWith("--")) throw new Error(`${key} requires a value.`);
    values[key.slice(2)] = value;
    index += 1;
  }
  return { command, values };
}

function required(values, key) {
  const value = values[key];
  if (!value) throw new Error(`--${key} is required.`);
  return value;
}

function main() {
  const { command, values } = parseArgs(process.argv.slice(2));
  if (command === "generate") {
    const releaseProvenance = values.provenance
      ? JSON.parse(readFileSync(resolve(values.provenance), "utf8"))
      : undefined;
    const manifest = generateReleaseManifest({
      treeRoot: required(values, "tree"),
      manifestPath: required(values, "manifest"),
      buildIdentifier: required(values, "build-id"),
      sourceRevision: required(values, "source-revision"),
      artifactIdentifier: required(values, "artifact-id"),
      privateKeyPath: required(values, "private-key"),
      releaseProvenance,
    });
    console.log(
      `Generated ${manifest.entry_count}-entry manifest for ${manifest.artifact_digest}.`,
    );
    return;
  }
  if (command === "verify") {
    const expectedReleaseProvenance = values.provenance
      ? JSON.parse(readFileSync(resolve(values.provenance), "utf8"))
      : undefined;
    const manifest = verifyReleaseManifest({
      treeRoot: required(values, "tree"),
      manifestPath: required(values, "manifest"),
      publicKeyPath: required(values, "public-key"),
      expectedBuildIdentifier: required(values, "build-id"),
      expectedSourceRevision: required(values, "source-revision"),
      expectedArtifactIdentifier: required(values, "artifact-id"),
      expectedReleaseProvenance,
    });
    console.log(
      `Verified ${manifest.entry_count}-entry manifest for ${manifest.artifact_digest}.`,
    );
    return;
  }
  if (command === "verify-subtree") {
    const result = verifyReleaseManifestSubtree({
      treeRoot: required(values, "tree"),
      manifestPath: required(values, "manifest"),
      publicKeyPath: required(values, "public-key"),
      pathPrefix: required(values, "prefix"),
      expectedBuildIdentifier: required(values, "build-id"),
      expectedSourceRevision: required(values, "source-revision"),
      expectedArtifactIdentifier: required(values, "artifact-id"),
    });
    console.log(
      `Verified installed ${result.subtree.path_prefix} subtree for ${result.subtree.artifact_digest}.`,
    );
    return;
  }
  throw new Error("Usage: release-manifest.mjs <generate|verify|verify-subtree> [options]");
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    main();
  } catch (error) {
    console.error(`OOMU RELEASE MANIFEST ERROR: ${error.message}`);
    process.exit(1);
  }
}
