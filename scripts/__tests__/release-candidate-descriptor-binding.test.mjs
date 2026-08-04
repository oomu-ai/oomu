import { generateKeyPairSync } from "node:crypto";
import {
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  inspectCanonicalReleaseBinding,
  signedCandidateDescriptorValue,
  verifyBoundSignedCandidateDescriptor,
} from "../release-candidate-descriptor.mjs";
import {
  artifactDigestForEntries,
  collectTreeEntries,
  generateReleaseManifest,
  subtreeDigestsForEntries,
} from "../release-manifest.mjs";

const temporaryDirectories = [];
const revision = "a".repeat(40);
const digest = "b".repeat(64);
const teamId = "R7AQ8287N6";
const bundleIdentifier = "ai.eldris.oomu.gpd";

afterEach(() => {
  while (temporaryDirectories.length) {
    rmSync(temporaryDirectories.pop(), { recursive: true, force: true });
  }
});

function releaseProvenance(candidateTree, applicationTreeDigest) {
  return {
    schemaVersion: 1,
    kind: "oomu.release-provenance",
    releaseVersion: {
      productVersion: "0.1.3",
      buildNumber: 8,
      intendedTag: "v0.1.3",
      channel: "public-beta",
      publicLabel: "OOMU 0.1.3 — Public Beta",
    },
    workflowSourceCommit: revision,
    releasePolicyId: "test-release-policy",
    releasePolicyDigest: "c".repeat(64),
    runnerIdentity: { architecture: "arm64" },
    actionCommitShas: { checkout: "d".repeat(40) },
    executableEvidence: {
      node: { executable: "/approved/node", sha256: "e".repeat(64) },
    },
    rustToolchain: { channel: "1.95.0", target: "aarch64-apple-darwin" },
    xcodeSdk: {
      developerDirectory: "/Applications/Xcode.app/Contents/Developer",
      sdkVersion: "26.5",
    },
    signedOutputDigest: artifactDigestForEntries(collectTreeEntries(candidateTree)),
    buildSignPhaseIsolated: false,
    releaseCandidateIntegrity: {
      kind: "oomu.release-candidate-integrity",
      schemaVersion: 2,
      reportSha256: "f".repeat(64),
      applicationTreeDigest,
      codeObjectCount: 1,
      bundleIdentifier,
      teamId,
      authority: `Developer ID Application: Eldris Inc (${teamId})`,
      buildNumber: 8,
      codeDirectoryHash: "1".repeat(40),
      designatedRequirementSha256: "2".repeat(64),
      hardenedRuntime: true,
      entitlementDigest: digest,
      gatekeeperAccepted: true,
      notarizationAccepted: true,
    },
  };
}

function boundDescriptorFixture() {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "oomu-candidate-binding-")));
  temporaryDirectories.push(root);
  const candidateTree = join(root, "candidate");
  const appPath = join(candidateTree, "OOMU.app");
  const containerPath = join(candidateTree, "OOMU-0.1.3.dmg");
  const entitlementReportPath = join(root, "entitlements.json");
  const manifestPath = join(root, "MANIFEST.json");
  const privateKeyPath = join(root, "private.pem");
  const publicKeyPath = join(root, "public.pem");
  mkdirSync(join(appPath, "Contents", "MacOS"), { recursive: true });
  writeFileSync(join(appPath, "Contents", "MacOS", "oomu"), "signed application");
  writeFileSync(containerPath, "signed and notarized DMG");
  writeFileSync(entitlementReportPath, JSON.stringify({
    application: { extracted: { canonical_sha256: digest } },
  }));
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  writeFileSync(privateKeyPath, privateKey.export({ format: "pem", type: "pkcs8" }));
  writeFileSync(publicKeyPath, publicKey.export({ format: "pem", type: "spki" }));
  const trustedPublicKeyHex = Buffer.from(
    publicKey.export({ format: "jwk" }).x,
    "base64url",
  ).toString("hex");
  const applicationTreeDigest = subtreeDigestsForEntries(
    collectTreeEntries(candidateTree),
  ).find((entry) => entry.path_prefix === "OOMU.app").artifact_digest;
  const provenance = releaseProvenance(candidateTree, applicationTreeDigest);
  generateReleaseManifest({
    treeRoot: candidateTree,
    manifestPath,
    buildIdentifier: "build-0.1.3-test",
    sourceRevision: revision,
    artifactIdentifier: "oomu-0.1.3-test",
    privateKeyPath,
    trustedPublicKeyHex,
    generatedAt: "2026-08-04T12:00:00.000Z",
    releaseProvenance: provenance,
  });
  const sourceIdentity = {
    sourceRevision: revision,
    sourceContentSha256: "3".repeat(64),
    worktreeStatusSha256: "4".repeat(64),
    worktreeClean: true,
  };
  const input = {
    appPath,
    containerPath,
    entitlementReportPath,
    manifestPath,
    publicKeyPath,
    releaseProvenancePath: join(root, "release-provenance.json"),
    appPrefix: "OOMU.app",
    expectedTeamId: teamId,
    expectedBundleIdentifier: bundleIdentifier,
    expectedBuildNumber: 8,
    expectedBuildIdentifier: "build-0.1.3-test",
    expectedArtifactIdentifier: "oomu-0.1.3-test",
  };
  writeFileSync(input.releaseProvenancePath, JSON.stringify(provenance));
  const binding = inspectCanonicalReleaseBinding(
    input,
    sourceIdentity,
    { trustedPublicKeyHex },
  );
  return {
    root,
    value: signedCandidateDescriptorValue(input, sourceIdentity, binding),
    trustedPublicKeyHex,
  };
}

describe("signed candidate descriptor security binding", () => {
  it("rejects arbitrary DMG substitution and identity-field tampering", () => {
    const fixture = boundDescriptorFixture();
    expect(verifyBoundSignedCandidateDescriptor(
      fixture.value,
      { trustedPublicKeyHex: fixture.trustedPublicKeyHex },
    )).toEqual(fixture.value);

    const rogueDirectory = join(fixture.root, "rogue");
    mkdirSync(rogueDirectory);
    const rogueDmg = join(rogueDirectory, "OOMU-0.1.3.dmg");
    writeFileSync(rogueDmg, "arbitrary same-name DMG");
    expect(() => verifyBoundSignedCandidateDescriptor(
      { ...fixture.value, containerPath: rogueDmg },
      { trustedPublicKeyHex: fixture.trustedPublicKeyHex },
    )).toThrow(/exact siblings/u);
    expect(() => verifyBoundSignedCandidateDescriptor(
      { ...fixture.value, expectedTeamId: "AAAAAAAAAA" },
      { trustedPublicKeyHex: fixture.trustedPublicKeyHex },
    )).toThrow(/canonical release provenance/u);
    expect(() => verifyBoundSignedCandidateDescriptor(
      { ...fixture.value, expectedBundleIdentifier: "example.invalid" },
      { trustedPublicKeyHex: fixture.trustedPublicKeyHex },
    )).toThrow(/canonical release provenance/u);
  });
});
