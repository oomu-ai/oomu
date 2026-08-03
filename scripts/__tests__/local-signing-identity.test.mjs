import { describe, expect, it } from "vitest";
import {
  DEVELOPMENT_BUNDLE_IDENTIFIER,
  PRODUCTION_BUNDLE_IDENTIFIER,
  localSigningIdentityPolicy,
} from "../local-signing-identity.mjs";

describe("local macOS signing identity", () => {
  it("allows ad-hoc signing only for the isolated development app", () => {
    expect(localSigningIdentityPolicy({
      appPath: "/tmp/OOMU Development.app",
      bundleIdentifier: DEVELOPMENT_BUNDLE_IDENTIFIER,
      signingIdentity: "-",
    })).toMatchObject({ adHoc: true, expectedBundleIdentifier: DEVELOPMENT_BUNDLE_IDENTIFIER });
  });

  it("rejects ad-hoc signing under the production identity", () => {
    expect(() => localSigningIdentityPolicy({
      appPath: "/tmp/OOMU.app",
      bundleIdentifier: PRODUCTION_BUNDLE_IDENTIFIER,
      signingIdentity: "-",
    })).toThrow("Ad-hoc signing requires OOMU Development.app");
  });

  it("allows the production app only with an explicit Developer ID identity", () => {
    expect(localSigningIdentityPolicy({
      appPath: "/tmp/OOMU.app",
      bundleIdentifier: PRODUCTION_BUNDLE_IDENTIFIER,
      signingIdentity: "Developer ID Application: Eldris (TEAM123456)",
      expectedTeamId: "TEAM123456",
    })).toMatchObject({ adHoc: false, expectedBundleIdentifier: PRODUCTION_BUNDLE_IDENTIFIER });
  });

  it("rejects a Developer ID identity on the development bundle", () => {
    expect(() => localSigningIdentityPolicy({
      appPath: "/tmp/OOMU Development.app",
      bundleIdentifier: DEVELOPMENT_BUNDLE_IDENTIFIER,
      signingIdentity: "Developer ID Application: Eldris (TEAM123456)",
      expectedTeamId: "TEAM123456",
    })).toThrow("Developer ID signing requires OOMU.app");
  });

  it("rejects Apple Development signing for the production app", () => {
    expect(() => localSigningIdentityPolicy({
      appPath: "/tmp/OOMU.app",
      bundleIdentifier: PRODUCTION_BUNDLE_IDENTIFIER,
      signingIdentity: "Apple Development: Eldris (TEAM123456)",
      expectedTeamId: "TEAM123456",
    })).toThrow("requires an explicit Developer ID Application identity");
  });

  it("rejects a different Developer ID team before signing the production app", () => {
    expect(() => localSigningIdentityPolicy({
      appPath: "/tmp/OOMU.app",
      bundleIdentifier: PRODUCTION_BUNDLE_IDENTIFIER,
      signingIdentity: "Developer ID Application: Someone Else (OTHER12345)",
      expectedTeamId: "TEAM123456",
    })).toThrow("does not match the reviewed Apple team");
  });
});
