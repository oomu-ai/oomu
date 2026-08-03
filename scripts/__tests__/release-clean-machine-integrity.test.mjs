import { describe, expect, it } from "vitest";
import { validateInstalledIntegrityEvidence } from "../release-clean-machine.mjs";

function fixture() {
  const namespace = "1234567890abcdef12345678";
  return {
    expected: {
      manifestPayloadSha256: "a".repeat(64),
      installedAppSubtree: {
        artifact_digest: `sha256:${"b".repeat(64)}`,
        entry_count: 25,
      },
      expectedBuildNumber: 2,
      expectedBundleIdentifier: "ai.eldris.oomu.gpd",
      expectedManifestSha256: "c".repeat(64),
      expectedAppTreeSha256: "b".repeat(64),
      expectedTeamId: "R7AQ8287N6",
      expectedDesignatedRequirementSha256: "d".repeat(64),
    },
    report: {
      installed_application_manifest_payload_sha256: "a".repeat(64),
      installed_application_subtree_digest: `sha256:${"b".repeat(64)}`,
      installed_application_entry_count: 25,
      nested_code_objects_verified: true,
      semantic_entitlements_verified: true,
      release_candidate_evidence: {
        bundleIdentifier: "ai.eldris.oomu.gpd",
        channel: "production",
        buildNumber: 2,
        appTreeSha256: "b".repeat(64),
        manifestSha256: "c".repeat(64),
        codesignVerified: true,
        gatekeeperAccepted: true,
        notarizationAccepted: true,
        nestedExecutablesVerified: true,
        installedTreeMatches: true,
        teamId: "R7AQ8287N6",
        designatedRequirementSha256: "d".repeat(64),
        beforeQualificationSha256: "b".repeat(64),
        afterQualificationSha256: "b".repeat(64),
      },
      runtime_identity_receipt: {
        kind: "runtime_identity",
        schemaVersion: 1,
        status: "verified",
        channel: "production",
        profileClass: "production",
        bundleIdentifier: "ai.eldris.oomu.gpd",
        teamId: "R7AQ8287N6",
        buildNumber: 2,
        applicationDataNamespace: "ai.eldris.oomu.gpd",
        keychainNamespaceClass: "production",
        codeDirectoryHash: "e".repeat(40),
        executableSha256: "f".repeat(64),
        strictSignatureValid: true,
        designatedRequirementSha256: "d".repeat(64),
        singleInstanceNamespace: namespace,
      },
      single_instance_receipt: {
        kind: "single_instance",
        schemaVersion: 2,
        decision: "primary_acquired",
        namespace,
        releaseChannel: "production",
        profileClass: "production",
        buildNumber: 2,
        codeDirectoryHash: "e".repeat(40),
        strictSignatureValid: true,
        holderPid: 4321,
      },
      process_cleanup_receipt: {
        kind: "exact_process_cleanup",
        schemaVersion: 1,
        status: "passed",
        synthetic: false,
        startedAt: "2026-07-30T12:00:00.000Z",
        completedAt: "2026-07-30T12:00:01.000Z",
        pid: 4321,
        parentPid: 1234,
        executableIdentitySha256: "a".repeat(64),
        executableSha256: "f".repeat(64),
        outcome: "graceful",
        forced: false,
        exitVerified: true,
      },
    },
  };
}

describe("post-install immutable candidate evidence", () => {
  it("accepts a complete real installed-tree and runtime receipt chain", () => {
    const { report, expected } = fixture();
    expect(validateInstalledIntegrityEvidence(report, expected)).toBe(true);
  });

  it("fails closed when an installed byte or exact-process receipt is missing", () => {
    const { report, expected } = fixture();
    report.installed_application_subtree_digest = `sha256:${"c".repeat(64)}`;
    expect(() => validateInstalledIntegrityEvidence(report, expected))
      .toThrow(/release_lab_integrity_evidence_incomplete/u);

    const next = fixture();
    delete next.report.process_cleanup_receipt;
    expect(() => validateInstalledIntegrityEvidence(next.report, next.expected))
      .toThrow(/release_lab_integrity_evidence_incomplete/u);
  });
});
