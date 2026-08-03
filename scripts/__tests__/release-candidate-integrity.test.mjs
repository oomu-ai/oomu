import { describe, expect, it } from "vitest";
import {
  candidateEvidenceSummary,
  parseCodeSignatureDetails,
  validateCodeEntitlementPolicy,
  validateEntitlementReport,
  verifiedApplicationBundleIdentifier,
} from "../release-candidate-integrity.mjs";

describe("immutable release-candidate evidence", () => {
  it("parses the exact Developer ID, hardened-runtime, and requirement identity", () => {
    const identity = parseCodeSignatureDetails([
      "Identifier=ai.eldris.oomu.gpd",
      "TeamIdentifier=R7AQ8287N6",
      "Authority=Developer ID Application: Eldris AI LLC (R7AQ8287N6)",
      "CDHash=1234567890abcdef1234567890abcdef12345678",
      "CodeDirectory v=20500 size=123 flags=0x10000(runtime) hashes=1 location=embedded",
      "designated => identifier \"ai.eldris.oomu.gpd\" and anchor apple generic",
    ].join("\n"));

    expect(identity).toMatchObject({
      bundleIdentifier: "ai.eldris.oomu.gpd",
      teamId: "R7AQ8287N6",
      authority: "Developer ID Application: Eldris AI LLC (R7AQ8287N6)",
      codeDirectoryHash: "1234567890abcdef1234567890abcdef12345678",
      hardenedRuntime: true,
    });
    expect(identity.designatedRequirementSha256).toMatch(/^[0-9a-f]{64}$/u);
  });

  it("accepts only a real passing semantic entitlement comparison", () => {
    const digest = "a".repeat(64);
    expect(validateEntitlementReport({
      kind: "oomu.final-signed-entitlement-snapshot",
      status: "passed",
      synthetic: false,
      application: {
        reviewed_policy: { canonical_sha256: digest },
        extracted: { canonical_sha256: digest },
      },
    })).toBe(digest);

    expect(() => validateEntitlementReport({
      kind: "oomu.final-signed-entitlement-snapshot",
      status: "passed",
      synthetic: true,
      application: {
        reviewed_policy: { canonical_sha256: digest },
        extracted: { canonical_sha256: digest },
      },
    })).toThrow(/semantically verified/iu);
  });

  it("rejects main or nested entitlements that differ from the reviewed policy", () => {
    const reviewed = "a".repeat(64);
    expect(() => validateCodeEntitlementPolicy({
      mainApplication: true,
      bundleIdentifier: "ai.eldris.oomu.gpd",
      expectedBundleIdentifier: "ai.eldris.oomu.gpd",
      entitlements: { keyCount: 1, canonicalSha256: "b".repeat(64) },
      reviewedEntitlementDigest: reviewed,
    })).toThrow(/differ from the reviewed policy/iu);

    expect(() => validateCodeEntitlementPolicy({
      mainApplication: false,
      bundleIdentifier: "ai.eldris.oomu.helper",
      expectedBundleIdentifier: "ai.eldris.oomu.gpd",
      entitlements: { keyCount: 1, canonicalSha256: "c".repeat(64) },
      reviewedEntitlementDigest: reviewed,
    })).toThrow(/unreviewed entitlements/iu);

    expect(() => validateCodeEntitlementPolicy({
      mainApplication: false,
      bundleIdentifier: "ai.eldris.oomu.helper",
      expectedBundleIdentifier: "ai.eldris.oomu.gpd",
      entitlements: { keyCount: 0, canonicalSha256: "d".repeat(64) },
      reviewedEntitlementDigest: reviewed,
    })).not.toThrow();
  });
});

describe("release-candidate evidence summary", () => {
  it("returns the complete stable packaged-candidate evidence contract", () => {
    const manifestPath = new URL(import.meta.url).pathname;
    const treeDigest = `sha256:${"b".repeat(64)}`;
    const summary = candidateEvidenceSummary({
      candidate: {
        application: {
          bundleIdentifier: "ai.eldris.oomu.gpd",
          buildNumber: "42",
          strictSignatureValid: true,
          gatekeeperAccepted: true,
          notarizationTicketValidated: true,
          teamId: "R7AQ8287N6",
        },
        container: {
          gatekeeperAccepted: true,
          notarizationTicketValidated: true,
        },
        codeObjects: [{ path: ".", designatedRequirementSha256: "d".repeat(64) }],
      },
      manifestPath,
      beforeQualification: treeDigest,
      afterQualification: treeDigest,
      nestedExecutablesVerified: true,
    });

    expect(Object.keys(summary)).toEqual([
      "bundleIdentifier",
      "channel",
      "buildNumber",
      "appTreeSha256",
      "manifestSha256",
      "codesignVerified",
      "gatekeeperAccepted",
      "notarizationAccepted",
      "nestedExecutablesVerified",
      "installedTreeMatches",
      "teamId",
      "designatedRequirementSha256",
      "beforeQualificationSha256",
      "afterQualificationSha256",
    ]);
    expect(summary).toMatchObject({
      bundleIdentifier: "ai.eldris.oomu.gpd",
      channel: "production",
      buildNumber: 42,
      appTreeSha256: "b".repeat(64),
      codesignVerified: true,
      gatekeeperAccepted: true,
      installedTreeMatches: true,
      teamId: "R7AQ8287N6",
    });
  });

  it("requires Gatekeeper to accept both the application and distribution container", () => {
    const manifestPath = new URL(import.meta.url).pathname;
    const summary = candidateEvidenceSummary({
      candidate: {
        application: {
          bundleIdentifier: "ai.eldris.oomu.gpd",
          buildNumber: "42",
          strictSignatureValid: true,
          gatekeeperAccepted: true,
          notarizationTicketValidated: true,
          teamId: "R7AQ8287N6",
        },
        container: {
          gatekeeperAccepted: false,
          notarizationTicketValidated: true,
        },
        codeObjects: [{ path: ".", designatedRequirementSha256: "d".repeat(64) }],
      },
      manifestPath,
      beforeQualification: `sha256:${"b".repeat(64)}`,
      afterQualification: `sha256:${"b".repeat(64)}`,
      nestedExecutablesVerified: true,
    });

    expect(summary.gatekeeperAccepted).toBe(false);
  });

  it("preserves the verified application identifier in release evidence", () => {
    expect(verifiedApplicationBundleIdentifier({
      path: ".",
      identifier: "ai.eldris.oomu.gpd",
    }, "ai.eldris.oomu.gpd")).toBe("ai.eldris.oomu.gpd");
    expect(() => verifiedApplicationBundleIdentifier({
      path: ".",
      identifier: "ai.eldris.oomu.gpd.development",
    }, "ai.eldris.oomu.gpd")).toThrow(/missing or unexpected/iu);
  });
});
