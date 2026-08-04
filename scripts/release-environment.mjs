import { readdirSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, resolve } from "node:path";
import process from "node:process";

const CARGO_ENCODED_FLAG_SEPARATOR = "\u001f";
const CANONICAL_RUST_PATHS = Object.freeze({
  repository: "/oomu/source",
  cargo: "/oomu/toolchains/cargo",
  rustup: "/oomu/toolchains/rustup",
  home: "/oomu/builder-home",
});

const SENSITIVE_CHILD_ENV = [
  "APPLE_CERTIFICATE",
  "APPLE_CERTIFICATE_PASSWORD",
  "APPLE_ID",
  "APPLE_PASSWORD",
  "APPLE_API_ISSUER",
  "APPLE_API_KEY",
  "APPLE_API_KEY_PATH",
  "APPLE_NOTARY_KEYCHAIN_PROFILE",
  "APPLE_SIGNING_IDENTITY",
  "APPLE_TEAM_ID",
  "OOMU_RELEASE_MANIFEST_PRIVATE_KEY_PATH",
  "OOMU_RELEASE_AUTHORIZATION_BASE64",
  "OOMU_OAUTH_SECRET_SCAN_CANARIES_BASE64",
  "TAURI_SIGNING_PRIVATE_KEY",
  "TAURI_SIGNING_PRIVATE_KEY_PATH",
  "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
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
  "APPLE_NOTARY_KEYCHAIN_PROFILE",
  "APPLE_SIGNING_IDENTITY",
  "APPLE_TEAM_ID",
  "OOMU_RELEASE_PIPELINE",
  "OOMU_BUILD_ID",
  "OOMU_SOURCE_REVISION",
  "OOMU_RELEASE_AUTHORIZATION_BASE64",
  "OOMU_RELEASE_MANIFEST_PUBLIC_KEY_PATH",
  "OOMU_OAUTH_SECRET_SCAN_CANARIES_BASE64",
  "OOMU_UPDATER_PUBLIC_KEY",
  "TAURI_SIGNING_PRIVATE_KEY",
  "TAURI_SIGNING_PRIVATE_KEY_PATH",
  "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
]);

const FORBIDDEN_RELEASE_ENVIRONMENT_NAMES = new Set([
  "NODE_ENV",
  "NODE_OPTIONS",
  "BASH_ENV",
  "ENV",
  "ZDOTDIR",
  "RUSTFLAGS",
  "CARGO_ENCODED_RUSTFLAGS",
  "CARGO_HOME",
  "CARGO_BUILD_TARGET",
  "CARGO_TARGET_DIR",
  "CFLAGS",
  "CXXFLAGS",
  "MACOSX_DEPLOYMENT_TARGET",
  "SDKROOT",
  "CC",
  "CXX",
  "AR",
  "LD",
  "RANLIB",
  "RUSTUP_HOME",
  "TAURI_ENV_TARGET_TRIPLE",
  "OOMU_PORTABLE_PYTHON_RELEASE",
  "OOMU_PORTABLE_PYTHON_VERSION",
  "OOMU_PORTABLE_PYTHON_URL",
  "OOMU_PORTABLE_PYTHON_ASSET",
  "OOMU_PORTABLE_PYTHON_SHA256",
]);

export function releaseToolchainHomeDirectory(toolchain) {
  const cargo = resolve(toolchain?.tools?.cargo?.executable ?? "");
  const rustBin = dirname(cargo);
  const toolchainDirectory = dirname(rustBin);
  const toolchainsDirectory = dirname(toolchainDirectory);
  const rustupHome = dirname(toolchainsDirectory);
  if (basename(cargo) !== "cargo" || basename(rustBin) !== "bin"
    || basename(toolchainsDirectory) !== "toolchains"
    || basename(rustupHome) !== ".rustup") {
    throw new Error("Pinned Rust toolchain does not identify a canonical release home.");
  }
  return dirname(rustupHome);
}

export function canonicalNativePathRemapConfiguration(
  checkoutRoot,
  source = process.env,
  pinnedHomeDirectory = null,
) {
  const repository = resolve(checkoutRoot);
  const ambientHome = resolve(source.HOME?.trim() || homedir());
  const home = resolve(pinnedHomeDirectory ?? ambientHome);
  if (ambientHome !== home) {
    throw new Error("Release HOME does not match the pinned Rust toolchain home.");
  }
  const mappings = [
    [repository, CANONICAL_RUST_PATHS.repository],
    [resolve(home, ".cargo"), CANONICAL_RUST_PATHS.cargo],
    [resolve(home, ".rustup"), CANONICAL_RUST_PATHS.rustup],
    [home, CANONICAL_RUST_PATHS.home],
  ]
    .filter(([from], index, values) =>
      values.findIndex(([candidate]) => candidate === from) === index)
    .sort(([left], [right]) => right.length - left.length);
  for (const [from, to] of mappings) {
    if (
      !from.startsWith("/")
      || !to.startsWith("/oomu/")
      || from.includes("=")
      || from.includes(CARGO_ENCODED_FLAG_SEPARATOR)
      || !/^\/[A-Za-z0-9._/-]+$/u.test(from)
      || !/^\/[A-Za-z0-9._/-]+$/u.test(to)
    ) {
      throw new Error("Canonical native path remapping requires bounded absolute paths.");
    }
  }
  const flags = [
    "--remap-path-scope=all",
    ...mappings.map(([from, to]) => `--remap-path-prefix=${from}=${to}`),
  ];
  const compilerFlags = mappings.flatMap(([from, to]) => [
    `-ffile-prefix-map=${from}=${to}`,
    `-fdebug-prefix-map=${from}=${to}`,
    `-fmacro-prefix-map=${from}=${to}`,
  ]);
  return { compilerFlags, mappings, rustFlags: flags };
}

export function canonicalNativePathRemapEnvironment(
  checkoutRoot,
  source = process.env,
  pinnedHomeDirectory = null,
) {
  const configuration = canonicalNativePathRemapConfiguration(
    checkoutRoot,
    source,
    pinnedHomeDirectory,
  );
  const compilerFlags = configuration.compilerFlags.join(" ");
  return {
    CARGO_ENCODED_RUSTFLAGS:
      configuration.rustFlags.join(CARGO_ENCODED_FLAG_SEPARATOR),
    CFLAGS: compilerFlags,
    CXXFLAGS: compilerFlags,
  };
}

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
