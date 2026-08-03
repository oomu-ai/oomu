import { generateKeyPairSync } from "node:crypto";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  symlinkSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  artifactDigestForEntries,
  collectTreeEntries,
  generateReleaseManifest,
  verifyReleaseManifest,
  verifyReleaseManifestSubtree,
  subtreeDigestsForEntries,
} from "../release-manifest.mjs";

let root;
let tree;
let manifestPath;
let privateKeyPath;
let publicKeyPath;
let trustedPublicKeyHex;

function useManifestFixture() {
  beforeEach(() => {
    root = mkdtempSync(join(tmpdir(), "oomu-manifest-test-"));
    tree = join(root, "candidate");
    manifestPath = join(root, "evidence", "MANIFEST.json");
    privateKeyPath = join(root, "private.pem");
    publicKeyPath = join(root, "public.pem");
    const { privateKey, publicKey } = generateKeyPairSync("ed25519");
    writeFileSync(
      privateKeyPath,
      privateKey.export({ format: "pem", type: "pkcs8" }),
    );
    writeFileSync(
      publicKeyPath,
      publicKey.export({ format: "pem", type: "spki" }),
    );
    trustedPublicKeyHex = Buffer.from(
      publicKey.export({ format: "jwk" }).x,
      "base64url",
    ).toString("hex");
    mkdirSync(join(tree, "Contents", "MacOS"), { recursive: true });
    writeFileSync(join(tree, "Contents", "MacOS", "oomu"), "binary-one");
    writeFileSync(join(tree, "Contents", "Info.plist"), "plist-two");
  });

  afterEach(() => rmSync(root, { recursive: true, force: true }));
}

function generate(releaseProvenance) {
  return generateReleaseManifest({
    treeRoot: tree,
    manifestPath,
    buildIdentifier: "build-214-test",
    sourceRevision: "a".repeat(40),
    artifactIdentifier: "oomu-test-artifact",
    privateKeyPath,
    trustedPublicKeyHex,
    generatedAt: "2026-07-09T12:00:00.000Z",
    releaseProvenance,
  });
}

function verify(expectedReleaseProvenance) {
  return verifyReleaseManifest({
    treeRoot: tree,
    manifestPath,
    publicKeyPath,
    expectedBuildIdentifier: "build-214-test",
    expectedSourceRevision: "a".repeat(40),
    expectedArtifactIdentifier: "oomu-test-artifact",
    trustedPublicKeyHex,
    expectedReleaseProvenance,
  });
}

function rewriteManifest(mutator) {
  chmodSync(manifestPath, 0o644);
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  mutator(manifest);
  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
}

describe("exact release-tree manifest", () => {
  useManifestFixture();

  it("atomically generates and verifies the exact candidate tree", () => {
    const manifest = generate();
    expect(manifest.entry_count).toBe(2);
    expect(manifest.artifact_digest).toMatch(/^sha256:[0-9a-f]{64}$/);
    expect(manifest.subtrees).toEqual([
      expect.objectContaining({ path_prefix: "Contents", entry_count: 2 }),
    ]);
    expect(verify().payload_sha256).toBe(manifest.payload_sha256);
    expect(statSync(manifestPath).mode & 0o222).toBe(0);
    expect(readdirSync(join(root, "evidence")).some((name) => name.includes(".tmp-"))).toBe(false);
  });

  it("rejects a one-byte mutation", () => {
    generate();
    writeFileSync(join(tree, "Contents", "MacOS", "oomu"), "binary-two");
    expect(verify).toThrow(/mismatch/i);
  });

  it("rejects a missing declared entry", () => {
    generate();
    unlinkSync(join(tree, "Contents", "Info.plist"));
    expect(verify).toThrow(/missing/i);
  });

  it("rejects an undeclared extra entry", () => {
    generate();
    writeFileSync(join(tree, "Contents", "extra"), "not declared");
    expect(verify).toThrow(/undeclared/i);
  });

  it("rejects count, signature, and source-revision mismatches", () => {
    generate();
    rewriteManifest((manifest) => {
      manifest.entry_count += 1;
    });
    expect(verify).toThrow(/entry count/i);

    generate();
    rewriteManifest((manifest) => {
      manifest.signature.value_base64 = Buffer.alloc(64).toString("base64");
    });
    expect(verify).toThrow(/signature/i);

    generate();
    expect(() =>
      verifyReleaseManifest({
        treeRoot: tree,
        manifestPath,
        publicKeyPath,
        expectedBuildIdentifier: "build-214-test",
        expectedSourceRevision: "b".repeat(40),
        expectedArtifactIdentifier: "oomu-test-artifact",
        trustedPublicKeyHex,
      }),
    ).toThrow(/source revision/i);
  });

  it("rejects a path-escaping symbolic link", () => {
    symlinkSync(tmpdir(), join(tree, "Contents", "escape"));
    expect(generate).toThrow(/escapes/i);
  });

  it("verifies an installed subtree and rejects an altered installed copy", () => {
    generate();
    const verifyInstalledContents = () => verifyReleaseManifestSubtree({
      treeRoot: join(tree, "Contents"),
      manifestPath,
      publicKeyPath,
      pathPrefix: "Contents",
      expectedBuildIdentifier: "build-214-test",
      expectedSourceRevision: "a".repeat(40),
      expectedArtifactIdentifier: "oomu-test-artifact",
      trustedPublicKeyHex,
    });

    expect(verifyInstalledContents().subtree.entry_count).toBe(2);
    writeFileSync(join(tree, "Contents", "MacOS", "oomu"), "altered-installed-copy");
    expect(verifyInstalledContents).toThrow(/mismatch/i);
  });

});

describe("signed release provenance manifest", () => {
  useManifestFixture();

  it("binds truthful non-isolated local provenance into the signed payload", () => {
    mkdirSync(join(tree, "OOMU.app", "Contents", "MacOS"), { recursive: true });
    writeFileSync(join(tree, "OOMU.app", "Contents", "MacOS", "oomu"), "signed-oomu");
    const applicationTreeDigest = subtreeDigestsForEntries(collectTreeEntries(tree))
      .find((subtree) => subtree.path_prefix === "OOMU.app").artifact_digest;
    const provenance = {
      kind: "oomu.release-provenance",
      workflowSourceCommit: "a".repeat(40),
      releasePolicyId: "test-release-policy",
      releasePolicyDigest: "b".repeat(64),
      runnerIdentity: { architecture: "arm64" },
      actionCommitShas: { checkout: "c".repeat(40) },
      executableEvidence: {
        node: { executable: "/approved/node", sha256: "d".repeat(64) },
      },
      rustToolchain: { channel: "1.95.0", target: "aarch64-apple-darwin" },
      xcodeSdk: {
        developerDirectory: "/Applications/Xcode.app/Contents/Developer",
        sdkVersion: "26.5",
      },
      signedOutputDigest: artifactDigestForEntries(collectTreeEntries(tree)),
      buildSignPhaseIsolated: false,
      releaseCandidateIntegrity: {
        kind: "oomu.release-candidate-integrity",
        schemaVersion: 1,
        reportSha256: "e".repeat(64),
        applicationTreeDigest,
        codeObjectCount: 1,
        bundleIdentifier: "ai.eldris.oomu.gpd",
        teamId: "R7AQ8287N6",
        authority: "Developer ID Application: Eldris AI LLC (R7AQ8287N6)",
        buildNumber: "42",
        codeDirectoryHash: "f".repeat(40),
        designatedRequirementSha256: "1".repeat(64),
        hardenedRuntime: true,
        entitlementDigest: "2".repeat(64),
        gatekeeperAccepted: true,
        notarizationAccepted: true,
      },
    };
    const manifest = generate(provenance);

    expect(manifest.release_provenance).toEqual(provenance);
    expect(verify(provenance).release_provenance.buildSignPhaseIsolated).toBe(false);
    expect(() => verify({ ...provenance, releasePolicyDigest: "d".repeat(64) }))
      .toThrow(/provenance does not match/u);
  });
});
