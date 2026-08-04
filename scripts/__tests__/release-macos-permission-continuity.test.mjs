import { describe, expect, it } from "vitest";
import {
  copyFileSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
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
      "com.apple.security.personal-information.addressbook": true,
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
});

describe("macOS permission continuity entitlement transitions", () => {
  it("allows only the reviewed Contacts entitlement addition from 0.1.2", () => {
    const reviewed = JSON.parse(readFileSync(
      join(root, "release", "macos-permission-continuity.snapshot.json"),
      "utf8",
    ));
    expect(reviewed.approved_n_plus_one_entitlement_transition).toEqual({
      from_product_version: "0.1.2",
      from_build_number: 7,
      to_product_version: "0.1.3",
      to_build_number: 8,
      additions: {
        "com.apple.security.personal-information.addressbook": true,
      },
    });

    const previous = signedSnapshot();
    previous.product_version = "0.1.2";
    previous.bundle_version = "7";
    previous.build_number = 7;
    delete previous.entitlements["com.apple.security.personal-information.addressbook"];
    const current = signedSnapshot();
    current.product_version = "0.1.3";
    current.bundle_version = "8";
    current.build_number = 8;

    expect(() => comparePermissionContinuity(previous, current, {
      requireBuildIncrease: true,
    })).toThrow("entitlements");
    expect(comparePermissionContinuity(previous, current, {
      approvedEntitlementTransition:
        reviewed.approved_n_plus_one_entitlement_transition,
      requireBuildIncrease: true,
    })).toBe(true);

    const wrongDestination = structuredClone(current);
    wrongDestination.product_version = "0.1.4";
    expect(() => comparePermissionContinuity(previous, wrongDestination, {
      approvedEntitlementTransition:
        reviewed.approved_n_plus_one_entitlement_transition,
      requireBuildIncrease: true,
    })).toThrow("entitlements");
  });

  it("rejects unreviewed, changed, and removed entitlements during the Contacts transition", () => {
    const previous = signedSnapshot();
    delete previous.entitlements["com.apple.security.personal-information.addressbook"];
    previous.product_version = "0.1.2";
    previous.bundle_version = "7";
    previous.build_number = 7;
    const approved = {
      from_product_version: "0.1.2",
      from_build_number: 7,
      to_product_version: "0.1.3",
      to_build_number: 8,
      additions: {
        "com.apple.security.personal-information.addressbook": true,
      },
    };

    const unreviewed = signedSnapshot();
    unreviewed.product_version = "0.1.3";
    unreviewed.bundle_version = "8";
    unreviewed.build_number = 8;
    unreviewed.entitlements["com.apple.security.files.user-selected.read-write"] = true;
    expect(() => comparePermissionContinuity(previous, unreviewed, {
      approvedEntitlementTransition: approved,
    })).toThrow("entitlements");

    const wrongValue = signedSnapshot();
    wrongValue.product_version = "0.1.3";
    wrongValue.bundle_version = "8";
    wrongValue.build_number = 8;
    wrongValue.entitlements["com.apple.security.personal-information.addressbook"] = false;
    expect(() => comparePermissionContinuity(previous, wrongValue, {
      approvedEntitlementTransition: approved,
    })).toThrow("entitlements");

    const removed = structuredClone(previous);
    removed.product_version = "0.1.3";
    removed.bundle_version = "8";
    removed.build_number = 8;
    delete removed.entitlements["com.apple.security.network.client"];
    expect(() => comparePermissionContinuity(previous, removed, {
      approvedEntitlementTransition: approved,
    })).toThrow("entitlements");
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
