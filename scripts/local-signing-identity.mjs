import { basename } from "node:path";

export const PRODUCTION_BUNDLE_IDENTIFIER = "ai.eldris.oomu.gpd";
export const DEVELOPMENT_BUNDLE_IDENTIFIER = "ai.eldris.oomu.gpd.development";

export function localSigningIdentityPolicy({
  appPath,
  bundleIdentifier,
  signingIdentity,
  expectedTeamId = "",
}) {
  const identity = signingIdentity?.trim() || "-";
  const adHoc = identity === "-";
  const expectedBundleIdentifier = adHoc
    ? DEVELOPMENT_BUNDLE_IDENTIFIER
    : PRODUCTION_BUNDLE_IDENTIFIER;
  const expectedBundleName = adHoc ? "OOMU Development.app" : "OOMU.app";

  if (!adHoc) {
    const teamId = identity.match(/^Developer ID Application:\s+.+\(([A-Z0-9]{10})\)$/u)?.[1];
    if (!teamId) {
      throw new Error(
        "Production OOMU requires an explicit Developer ID Application identity.",
      );
    }
    if (!expectedTeamId || teamId !== expectedTeamId) {
      throw new Error("The Developer ID identity does not match the reviewed Apple team.");
    }
  }

  if (basename(appPath) !== expectedBundleName) {
    throw new Error(
      `${adHoc ? "Ad-hoc" : "Developer ID"} signing requires ${expectedBundleName}.`,
    );
  }
  if (bundleIdentifier !== expectedBundleIdentifier) {
    throw new Error(
      `${adHoc ? "Ad-hoc" : "Developer ID"} signing cannot use bundle identifier ${bundleIdentifier}. `
      + `Expected ${expectedBundleIdentifier}.`,
    );
  }

  return { adHoc, expectedBundleIdentifier, expectedBundleName, signingIdentity: identity };
}
