import { describe, expect, it } from "vitest";
import { copyFileSync, mkdtempSync, mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { comparePermissionContinuity } from "../release-gates/macos-permission-continuity.mjs";
import { permissionContinuityPrerequisite } from "../release.mjs";

const root = resolve(import.meta.dirname, "..", "..");

function lineageFixture() {
  const directory = mkdtempSync(join(tmpdir(), "oomu-permission-lineage-"));
  const lineage = join(directory, "lineage.json");
  copyFileSync(join(root, "release", "macos-permission-lineage.json"), lineage);
  return { directory, lineage };
}

function signedSnapshot() {
  return {
    schema_version: 2,
    kind: "oomu.macos-permission-continuity.signed",
    bundle_identifier: "ai.eldris.oomu.gpd",
    product_version: "0.1.0",
    bundle_version: "1",
    build_number: 1,
    build_number_source: "bundle",
    main_executable: "oomu",
    usage_description_keys: [
      "NSAppleEventsUsageDescription",
      "NSCalendarsFullAccessUsageDescription",
    ],
    entitlements: {
      "com.apple.security.automation.apple-events": true,
      "com.apple.security.network.client": true,
      "com.apple.security.personal-information.calendars": true,
    },
    application_signature: {
      identifier: "ai.eldris.oomu.gpd",
      team_identifier: "TEAM123456",
      authority_chain: ["Developer ID Application: OOMU"],
      designated_requirement: "identifier ai.eldris.oomu.gpd and anchor apple generic",
      designated_requirement_sha256: "a".repeat(64),
      hardened_runtime: true,
    },
    helpers: [
      {
        name: "local_infer",
        identifier: "local_infer",
        team_identifier: "TEAM123456",
        authority_chain: ["Developer ID Application: OOMU"],
        designated_requirement: "identifier local_infer and anchor apple generic",
        designated_requirement_sha256: "b".repeat(64),
        hardened_runtime: true,
      },
    ],
    strict_code_signature: "valid",
  };
}

describe("macOS permission continuity release gate", () => {
  it("uses the immutable reviewed lineage only for the exact first release", () => {
    const { directory, lineage } = lineageFixture();
    const absentPrevious = join(directory, "missing.app");
    const releaseVersion = { productVersion: "0.1.0", buildNumber: 1 };
    expect(permissionContinuityPrerequisite({}, {
      defaultPreviousApp: absentPrevious,
      lineageEvidencePath: lineage,
      releaseVersion,
      bundleIdentifier: "ai.eldris.oomu.gpd",
    })).toEqual({
      firstSignedRelease: true,
      previousSignedApp: null,
      lineageEvidencePath: lineage,
    });

    expect(() => permissionContinuityPrerequisite({}, {
      defaultPreviousApp: absentPrevious,
      lineageEvidencePath: lineage,
      releaseVersion: { productVersion: "0.1.1", buildNumber: 2 },
      bundleIdentifier: "ai.eldris.oomu.gpd",
    })).toThrow("not the reviewed first signed release");
  });

  it("rejects environment assertions and changed lineage records", () => {
    const { directory, lineage } = lineageFixture();
    expect(() => permissionContinuityPrerequisite(
      { OOMU_FIRST_SIGNED_RELEASE: "1" },
      { defaultPreviousApp: join(directory, "missing.app"), lineageEvidencePath: lineage },
    )).toThrow("no longer accepted");

    writeFileSync(lineage, "{}\n");
    expect(() => permissionContinuityPrerequisite({}, {
      defaultPreviousApp: join(directory, "missing.app"),
      lineageEvidencePath: lineage,
    })).toThrow("permission lineage changed");
  });

  it("requires and accepts a real previous app after the bootstrap release", () => {
    const { directory, lineage } = lineageFixture();
    const previous = join(directory, "OOMU.app");
    mkdirSync(previous);
    expect(permissionContinuityPrerequisite({}, {
      defaultPreviousApp: previous,
      lineageEvidencePath: lineage,
      releaseVersion: { productVersion: "0.1.1", buildNumber: 2 },
    })).toMatchObject({ firstSignedRelease: false, previousSignedApp: realpathSync(previous) });
  });

  it("accepts an unchanged signed identity", () => {
    const previous = signedSnapshot();
    const current = structuredClone(previous);
    expect(comparePermissionContinuity(previous, current)).toBe(true);
  });

  it("requires a newer signed-candidate build number for an update", () => {
    const previous = signedSnapshot();
    const current = structuredClone(previous);
    expect(() => comparePermissionContinuity(previous, current, {
      requireBuildIncrease: true,
    })).toThrow("build_number");
    current.build_number = 2;
    current.bundle_version = "2";
    expect(comparePermissionContinuity(previous, current, {
      requireBuildIncrease: true,
    })).toBe(true);
  });

  it("binds the Apple bundle version to the signed-candidate build number", () => {
    const previous = signedSnapshot();
    const current = structuredClone(previous);
    current.bundle_version = "0.1.0";
    expect(() => comparePermissionContinuity(previous, current)).toThrow("bundle_version");
  });

  it("allows only explicitly reviewed usage-description additions", () => {
    const previous = signedSnapshot();
    const current = structuredClone(previous);
    current.usage_description_keys.push("NSRemindersFullAccessUsageDescription");

    expect(() => comparePermissionContinuity(previous, current)).toThrow(
      "usage_description_keys",
    );
    expect(comparePermissionContinuity(previous, current, {
      approvedUsageKeyAdditions: ["NSRemindersFullAccessUsageDescription"],
    })).toBe(true);

    const unreviewed = structuredClone(previous);
    unreviewed.usage_description_keys.push("NSBluetoothAlwaysUsageDescription");
    expect(() => comparePermissionContinuity(previous, unreviewed, {
      approvedUsageKeyAdditions: ["NSRemindersFullAccessUsageDescription"],
    })).toThrow("usage_description_keys");
  });

  it.each([
    ["bundle identifier", (value) => { value.bundle_identifier = "ai.eldris.oomu.changed"; }, "bundle_identifier"],
    ["main executable", (value) => { value.main_executable = "changed"; }, "main_executable"],
    ["entitlements", (value) => { value.entitlements["com.apple.security.network.client"] = false; }, "entitlements"],
    ["Team ID", (value) => { value.application_signature.team_identifier = "OTHERTEAM"; }, "application_signature.team_identifier"],
    ["designated requirement", (value) => { value.application_signature.designated_requirement = "changed"; }, "application_signature.designated_requirement"],
    ["hardened runtime", (value) => { value.application_signature.hardened_runtime = false; }, "application_signature.hardened_runtime"],
    ["helper inventory", (value) => { value.helpers = []; }, "helper_executables"],
    ["helper signature", (value) => { value.helpers[0].team_identifier = "OTHERTEAM"; }, "helper.local_infer.team_identifier"],
  ])("rejects a changed %s", (_label, mutate, marker) => {
    const previous = signedSnapshot();
    const current = structuredClone(previous);
    mutate(current);
    expect(() => comparePermissionContinuity(previous, current)).toThrow(marker);
  });
});
