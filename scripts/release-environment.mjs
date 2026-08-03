import { readdirSync } from "node:fs";
import { resolve } from "node:path";
import process from "node:process";

const SENSITIVE_CHILD_ENV = [
  "APPLE_CERTIFICATE",
  "APPLE_CERTIFICATE_PASSWORD",
  "APPLE_ID",
  "APPLE_PASSWORD",
  "APPLE_API_ISSUER",
  "APPLE_API_KEY",
  "APPLE_API_KEY_PATH",
  "APPLE_SIGNING_IDENTITY",
  "APPLE_TEAM_ID",
  "OOMU_RELEASE_MANIFEST_PRIVATE_KEY_PATH",
  "OOMU_RELEASE_AUTHORIZATION_BASE64",
  "OOMU_OAUTH_SECRET_SCAN_CANARIES_BASE64",
];

const RELEASE_CHILD_ENV_ALLOWLIST = Object.freeze([
  "PATH",
  "HOME",
  "TMPDIR",
  "TMP",
  "TEMP",
  "USER",
  "LOGNAME",
  "SHELL",
  "LANG",
  "LANGUAGE",
  "LC_ALL",
  "LC_CTYPE",
  "TERM",
  "CI",
  "GITHUB_ACTIONS",
  "DEVELOPER_DIR",
  "HTTP_PROXY",
  "HTTPS_PROXY",
  "NO_PROXY",
  "http_proxy",
  "https_proxy",
  "no_proxy",
  "SSL_CERT_FILE",
  "SSL_CERT_DIR",
  "CURL_CA_BUNDLE",
  "OOMU_SLACK_OAUTH_BROKER_URL",
  "OOMU_SLACK_OAUTH_BROKER_CERT_SHA256",
  "OOMU_MICROSOFT_OAUTH_CLIENT_ID",
]);

const RELEASE_CHILD_OVERRIDE_ALLOWLIST = new Set([
  "APPLE_CERTIFICATE",
  "APPLE_CERTIFICATE_PASSWORD",
  "APPLE_ID",
  "APPLE_PASSWORD",
  "APPLE_API_ISSUER",
  "APPLE_API_KEY",
  "APPLE_API_KEY_PATH",
  "APPLE_SIGNING_IDENTITY",
  "APPLE_TEAM_ID",
  "OOMU_RELEASE_PIPELINE",
  "OOMU_BUILD_ID",
  "OOMU_SOURCE_REVISION",
  "OOMU_RELEASE_AUTHORIZATION_BASE64",
  "OOMU_RELEASE_MANIFEST_PUBLIC_KEY_PATH",
  "OOMU_OAUTH_SECRET_SCAN_CANARIES_BASE64",
]);

const FORBIDDEN_RELEASE_ENVIRONMENT_NAMES = new Set([
  "NODE_ENV",
  "NODE_OPTIONS",
  "BASH_ENV",
  "ENV",
  "ZDOTDIR",
  "RUSTFLAGS",
  "CARGO_ENCODED_RUSTFLAGS",
  "CARGO_BUILD_TARGET",
  "MACOSX_DEPLOYMENT_TARGET",
  "SDKROOT",
  "CC",
  "CXX",
  "AR",
  "LD",
  "RANLIB",
  "TAURI_ENV_TARGET_TRIPLE",
  "OOMU_PORTABLE_PYTHON_RELEASE",
  "OOMU_PORTABLE_PYTHON_VERSION",
  "OOMU_PORTABLE_PYTHON_URL",
  "OOMU_PORTABLE_PYTHON_ASSET",
  "OOMU_PORTABLE_PYTHON_SHA256",
]);

function releaseEnvironmentOverrideNames(environment) {
  return Object.keys(environment).filter(
    (name) =>
      name.startsWith("NEXT_PUBLIC_") ||
      name.startsWith("TAURI_ENV_") ||
      FORBIDDEN_RELEASE_ENVIRONMENT_NAMES.has(name),
  );
}

export function assertNoReleaseEnvironmentOverrides(environment = process.env) {
  const forbidden = releaseEnvironmentOverrideNames(environment).sort();
  if (forbidden.length > 0) {
    throw new Error(
      `Release-affecting environment overrides are prohibited: ${forbidden.join(", ")}.`,
    );
  }
}

export function assertNoRepositoryDotenvFiles(checkoutRoot) {
  const dotenvInputs = readdirSync(resolve(checkoutRoot), { withFileTypes: true })
    .map((entry) => entry.name)
    .filter((name) => name.startsWith(".env"))
    .sort();
  if (dotenvInputs.length > 0) {
    throw new Error(
      `Repository-root .env* inputs are prohibited in a release build: ${dotenvInputs.join(", ")}.`,
    );
  }
}

export function createSanitizedChildEnvironment(
  overrides,
  source,
  immutableReleaseToolchain,
) {
  assertNoReleaseEnvironmentOverrides(source);
  const unsupportedOverrides = Object.keys(overrides).filter(
    (name) => !RELEASE_CHILD_OVERRIDE_ALLOWLIST.has(name),
  );
  if (unsupportedOverrides.length > 0) {
    throw new Error(
      `Unreviewed release child-environment overrides are prohibited: ${unsupportedOverrides.sort().join(", ")}.`,
    );
  }
  const environment = Object.fromEntries(
    RELEASE_CHILD_ENV_ALLOWLIST
      .filter((name) => source[name] !== undefined)
      .map((name) => [name, source[name]]),
  );
  for (const name of SENSITIVE_CHILD_ENV) delete environment[name];
  if (immutableReleaseToolchain) {
    environment.PATH = immutableReleaseToolchain.minimalPath;
    environment.DEVELOPER_DIR = immutableReleaseToolchain.runner.xcode.developerDirectory;
  }
  return {
    MACOSX_DEPLOYMENT_TARGET:
      immutableReleaseToolchain?.policy.deploymentTarget ?? "14.0",
    ...environment,
    ...overrides,
  };
}

export function externalHarnessEnvironment(overrides = {}) {
  const environment = {};
  for (const name of [
    "PATH",
    "HOME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "CI",
    "OOMU_RELEASE_LAB_URL",
    "OOMU_RELEASE_LAB_CLIENT_CERT_PATH",
    "OOMU_RELEASE_LAB_CLIENT_KEY_PATH",
    "OOMU_RELEASE_LAB_CA_PATH",
  ]) {
    if (process.env[name]) environment[name] = process.env[name];
  }
  return { ...environment, ...overrides };
}
