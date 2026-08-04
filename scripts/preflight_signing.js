#!/usr/bin/env node

let fs;
let path;
let childProcess;

let repoRoot;
let examplePath;
let tauriConfigPath;

const signingIdentityVars = ["APPLE_SIGNING_IDENTITY"];
const certificateImportVars = ["APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD"];
const appleIdNotarizationVars = ["APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID"];
const apiNotarizationVars = ["APPLE_API_ISSUER", "APPLE_API_KEY", "APPLE_API_KEY_PATH"];
const keychainNotarizationVars = ["APPLE_NOTARY_KEYCHAIN_PROFILE"];
const trackedVars = [
  ...signingIdentityVars,
  ...certificateImportVars,
  ...appleIdNotarizationVars,
  ...apiNotarizationVars,
  ...keychainNotarizationVars,
];

function isPresent(value) {
  return typeof value === "string" && value.trim().length > 0;
}

function getActiveEnvValues() {
  return Object.fromEntries(
    trackedVars
      .filter((name) => isPresent(process.env[name]))
      .map((name) => [name, process.env[name]])
  );
}

function readBundleIdentifier() {
  try {
    const config = JSON.parse(fs.readFileSync(tauriConfigPath, "utf8"));
    return isPresent(config.identifier) ? config.identifier : "not configured";
  } catch {
    return "unable to read src-tauri/tauri.conf.json";
  }
}

function missingVars(requiredVars, values) {
  return requiredVars.filter((name) => !isPresent(values[name]));
}

function hasAll(requiredVars, values) {
  return missingVars(requiredVars, values).length === 0;
}

function buildStatus(values) {
  const hasSigningIdentity = hasAll(signingIdentityVars, values);
  const hasAppleIdNotarization = hasAll(appleIdNotarizationVars, values);
  const hasApiNotarization = hasAll(apiNotarizationVars, values);
  const hasKeychainNotarization = hasAll(keychainNotarizationVars, values);
  const setCertificateVars = certificateImportVars.filter((name) => isPresent(values[name]));
  const hasPartialCertificateImport =
    setCertificateVars.length > 0 && setCertificateVars.length < certificateImportVars.length;

  return {
    hasSigningIdentity,
    hasAppleIdNotarization,
    hasApiNotarization,
    hasKeychainNotarization,
    hasPartialCertificateImport,
    missingSigningIdentity: missingVars(signingIdentityVars, values),
    missingAppleIdNotarization: missingVars(appleIdNotarizationVars, values),
    missingApiNotarization: missingVars(apiNotarizationVars, values),
    missingKeychainNotarization: missingVars(keychainNotarizationVars, values),
    missingCertificateImport: missingVars(certificateImportVars, values),
  };
}

function printFailure({ activeValues, values, status }) {
  const bundleIdentifier = readBundleIdentifier();
  const red = process.stderr.isTTY ? "\x1b[31m" : "";
  const yellow = process.stderr.isTTY ? "\x1b[33m" : "";
  const reset = process.stderr.isTTY ? "\x1b[0m" : "";

  console.error(`${red}OOMU RELEASE ERROR: Apple signing environment is incomplete.${reset}`);
  console.error("");
  console.error(
    "The production macOS build needs signing and notarization inputs before Tauri can build app and DMG bundles."
  );
  console.error("");
  console.error("Detected source:");
  console.error(
    `  - active environment: ${
      Object.keys(activeValues).length > 0 ? "contains signing-related names" : "none found"
    }`
  );
  console.error("");
  console.error("Required values:");
  console.error(
    "  - APPLE_SIGNING_IDENTITY: Developer ID Application identity or SHA-1 hash from `security find-identity -v -p codesigning`."
  );
  console.error(
    "  - APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID: Apple ID notarization flow with an app-specific password."
  );
  console.error(
    "  - Or APPLE_API_ISSUER, APPLE_API_KEY, APPLE_API_KEY_PATH: App Store Connect API notarization flow."
  );
  console.error(
    "  - Or APPLE_NOTARY_KEYCHAIN_PROFILE: an existing notarytool profile stored in the macOS Keychain."
  );
  console.error(
    "  - APPLE_CERTIFICATE and APPLE_CERTIFICATE_PASSWORD: required together when importing a .p12 certificate in CI."
  );
  console.error(`  - Bundle identifier: ${bundleIdentifier} from src-tauri/tauri.conf.json.`);
  console.error("");
  console.error("Missing from the active environment:");
  if (!status.hasSigningIdentity) {
    console.error(`  - ${status.missingSigningIdentity.join(", ")}`);
  }
  if (!status.hasAppleIdNotarization && !status.hasApiNotarization && !status.hasKeychainNotarization) {
    console.error(
      `  - Notarization credentials. Provide ${keychainNotarizationVars.join(", ")}, ${appleIdNotarizationVars.join(", ")}, or ${apiNotarizationVars.join(", ")}.`
    );
  }
  if (status.hasPartialCertificateImport) {
    console.error(`  - Complete the certificate import pair: ${status.missingCertificateImport.join(", ")}.`);
  }
  if (
    status.hasSigningIdentity &&
    (status.hasAppleIdNotarization || status.hasApiNotarization || status.hasKeychainNotarization) &&
    !status.hasPartialCertificateImport
  ) {
    console.error("  - No individual required values are missing, but the configuration did not validate.");
  }
  console.error("");
  console.error(`${yellow}To resolve:${reset}`);
  console.error("  1. Export the required variables explicitly in your reviewed shell environment or CI secret store.");
  console.error(`  2. Use ${path.relative(repoRoot, examplePath)} only as a list of variable names; it is never executed or loaded.`);
  console.error("  3. Run npm run build:prod from that already-provisioned environment.");
  console.error("");
  console.error("No secret values were printed.");

  if (Object.keys(values).length > 0) {
    console.error("Validated source names only: active environment.");
  }
}

async function loadNodeModules() {
  fs = await import("node:fs");
  path = await import("node:path");
  childProcess = await import("node:child_process");
  repoRoot = path.resolve(__dirname, "..");
  examplePath = path.join(__dirname, "sign_env.sh.example");
  tauriConfigPath = path.join(repoRoot, "src-tauri", "tauri.conf.json");
}

async function main() {
  await loadNodeModules();

  const activeValues = getActiveEnvValues();
  const values = activeValues;
  const status = buildStatus(values);
  const notarizationReady =
    status.hasAppleIdNotarization || status.hasApiNotarization || status.hasKeychainNotarization;
  const certificateImportReady = !status.hasPartialCertificateImport;
  const teamId = values.APPLE_TEAM_ID;
  const teamIdReady = isPresent(teamId) && /^[A-Z0-9]{10}$/.test(teamId);

  if (!status.hasSigningIdentity || !notarizationReady || !certificateImportReady || !teamIdReady) {
    printFailure({ activeValues, values, status });
    if (!teamIdReady) {
      console.error("APPLE_TEAM_ID must be the 10-character Team ID bound to the signing identity.");
    }
    process.exit(1);
  }

  const securityEnvironment = { ...process.env };
  for (const name of trackedVars) {
    delete securityEnvironment[name];
  }

  const identities = childProcess.spawnSync(
    "/usr/bin/security",
    ["find-identity", "-v", "-p", "codesigning"],
    { encoding: "utf8", env: securityEnvironment },
  );
  if (identities.error || identities.status !== 0) {
    console.error("Signing preflight could not query the macOS codesigning keychain.");
    process.exit(1);
  }
  const configuredIdentity = values.APPLE_SIGNING_IDENTITY.trim();
  const matchingLine = identities.stdout
    .split(/\r?\n/)
    .find((line) => line.includes(configuredIdentity));
  if (!matchingLine) {
    console.error("Signing preflight did not find APPLE_SIGNING_IDENTITY in the codesigning keychain.");
    console.error("No certificate or secret values were printed.");
    process.exit(1);
  }
  if (!matchingLine.includes(`(${teamId})`)) {
    console.error("Signing preflight found the identity, but its certificate Team ID does not match APPLE_TEAM_ID.");
    console.error("No certificate or secret values were printed.");
    process.exit(1);
  }

  if (status.hasKeychainNotarization) {
    const profileCheck = childProcess.spawnSync(
      "/usr/bin/xcrun",
      [
        "notarytool",
        "history",
        "--keychain-profile",
        values.APPLE_NOTARY_KEYCHAIN_PROFILE.trim(),
        "--output-format",
        "json",
      ],
      { encoding: "utf8", env: securityEnvironment },
    );
    if (profileCheck.error || profileCheck.status !== 0) {
      console.error("Signing preflight could not authenticate the configured Apple notary Keychain profile.");
      console.error("No credential values were printed.");
      process.exit(1);
    }
  }

  const notarizationMode = status.hasKeychainNotarization
    ? "macOS Keychain notarization profile"
    : status.hasAppleIdNotarization
      ? "Apple ID notarization"
      : "App Store Connect API notarization";
  console.log(
    `Signing preflight passed: found ${notarizationMode} inputs and APPLE_SIGNING_IDENTITY via the active environment.`
  );
}

main().catch((error) => {
  console.error(`Signing preflight failed before validation: ${error.message}`);
  process.exit(1);
});
