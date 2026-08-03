import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  signedCandidateDescriptorValue,
  RELEASE_CANDIDATE_DESCRIPTOR_KIND,
  RELEASE_CANDIDATE_DESCRIPTOR_SCHEMA_VERSION,
} from "../release-candidate-descriptor.mjs";

const digest = "a".repeat(64);
const revision = "b".repeat(40);
const releaseBinding = {
  kind: "oomu.canonical-release-entrypoint-binding",
  schemaVersion: 1,
  entrypoint: "scripts/release.mjs",
  releaseProvenancePath: "/private/tmp/release-provenance.json",
  releaseProvenanceSha256: digest,
  releaseProvenanceStableSha256: digest,
  manifestSha256: digest,
  manifestPayloadSha256: digest,
  signedOutputDigest: `sha256:${digest}`,
};
const input = {
  appPath: "/private/tmp/OOMU.app",
  containerPath: "/private/tmp/OOMU.dmg",
  entitlementReportPath: "/private/tmp/entitlements.json",
  manifestPath: "/private/tmp/manifest.json",
  publicKeyPath: "/private/tmp/public.pem",
  appPrefix: "OOMU.app",
  expectedTeamId: "R7AQ8287N6",
  expectedBundleIdentifier: "ai.eldris.oomu.gpd",
  expectedBuildNumber: 302,
  expectedBuildIdentifier: "0.1.1+302",
  expectedArtifactIdentifier: "oomu-macos-302",
};

describe("signed-candidate descriptor", () => {
  it("is materialized only by the canonical release entrypoint", () => {
    const releaseSource = readFileSync(
      resolve(import.meta.dirname, "../release.mjs"), "utf8",
    );
    expect(releaseSource).toContain("writeCanonicalSignedCandidateDescriptor");
    expect(releaseSource).toContain("releaseEvidence.releaseProvenancePath");
    expect(releaseSource).toContain("OOMU_SIGNED_CANDIDATE_DESCRIPTOR=");
  });

  it("binds the candidate to one exact clean source identity", () => {
    expect(signedCandidateDescriptorValue(input, {
      sourceRevision: revision,
      sourceContentSha256: digest,
      worktreeStatusSha256: digest,
      worktreeClean: true,
    }, releaseBinding)).toMatchObject({
      kind: RELEASE_CANDIDATE_DESCRIPTOR_KIND,
      schemaVersion: RELEASE_CANDIDATE_DESCRIPTOR_SCHEMA_VERSION,
      expectedSourceRevision: revision,
      sourceContentSha256: digest,
      sourceWorktreeClean: true,
    });
  });

  it("refuses to describe a dirty or unidentified source tree", () => {
    expect(() => signedCandidateDescriptorValue(input, {
      sourceRevision: revision,
      sourceContentSha256: digest,
      worktreeStatusSha256: digest,
      worktreeClean: false,
    }, releaseBinding)).toThrow(/clean, exact source identity/u);
  });

  it("rejects a manually assembled descriptor with no canonical release binding", () => {
    expect(() => signedCandidateDescriptorValue(input, {
      sourceRevision: revision,
      sourceContentSha256: digest,
      worktreeStatusSha256: digest,
      worktreeClean: true,
    })).toThrow(/canonical cryptographic release provenance/u);
  });
});
