import { createHash } from "node:crypto";
import { lstatSync, readFileSync, readdirSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";

export const name = "bundle_resource_inventory";

export const SUPPORTED_LOCALE_FILES = Object.freeze([
  "de-DE.json",
  "en-US.json",
  "es-ES.json",
  "fr-FR.json",
  "id-ID.json",
  "ja-JP.json",
  "pt-BR.json",
  "ru-RU.json",
  "uk-UA.json",
  "vi-VN.json",
  "zh-CN.json",
  "zh-TW.json",
]);

const LOCALIZED_INFO_PLIST_FILES = Object.freeze([
  "de.lproj/InfoPlist.strings",
  "en.lproj/InfoPlist.strings",
  "es.lproj/InfoPlist.strings",
  "fr.lproj/InfoPlist.strings",
  "id.lproj/InfoPlist.strings",
  "ja.lproj/InfoPlist.strings",
  "pt-BR.lproj/InfoPlist.strings",
  "ru.lproj/InfoPlist.strings",
  "uk.lproj/InfoPlist.strings",
  "vi.lproj/InfoPlist.strings",
  "zh-Hans.lproj/InfoPlist.strings",
  "zh-Hant.lproj/InfoPlist.strings",
]);

const REQUIRED_RESOURCE_FILES = Object.freeze([
  "Contents/Resources/Assets.car",
  "Contents/Resources/OOMU.icns",
  "Contents/Resources/_up_/THIRD_PARTY_NOTICES.md",
  "Contents/Resources/resources/mcp/mcp_applescript.py",
  "Contents/Resources/resources/mcp/mcp_search.py",
  "Contents/Resources/resources/python/.oomu-python-build.json",
  "Contents/Resources/resources/python/.oomu-python-native-manifest.json",
  "Contents/Resources/resources/python/bin/python3.10",
  ...LOCALIZED_INFO_PLIST_FILES.map((path) => `Contents/Resources/${path}`),
]);

const LOCALE_RESOURCE_PREFIX = "../src/locales/";

function sorted(values) {
  return [...values].sort((left, right) => left.localeCompare(right));
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export function configuredLocaleResources(config) {
  return (config?.bundle?.resources ?? [])
    .filter((path) => typeof path === "string" && path.startsWith(LOCALE_RESOURCE_PREFIX));
}

export function validateConfiguredLocaleResources(config) {
  const actual = configuredLocaleResources(config);
  const expected = SUPPORTED_LOCALE_FILES.map((file) => `${LOCALE_RESOURCE_PREFIX}${file}`);
  if (JSON.stringify(sorted(actual)) !== JSON.stringify(sorted(expected))) {
    throw new Error(
      "Production locale resources must contain exactly the 12 supported JSON catalogs.",
    );
  }
  if (actual.some((path) => !path.endsWith(".json") || /(?:^|[._-])(?:test|spec)(?:[._-]|$)/iu.test(path))) {
    throw new Error("Production locale resources contain a non-JSON or test source file.");
  }
  return sorted(actual);
}

function filesUnder(root) {
  const files = [];
  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      const metadata = lstatSync(path);
      if (metadata.isSymbolicLink()) continue;
      if (metadata.isDirectory()) visit(path);
      else if (metadata.isFile()) files.push(path);
    }
  };
  visit(root);
  return files;
}

function requireNonemptyFile(app, relativePath) {
  const path = join(app, ...relativePath.split("/"));
  const metadata = lstatSync(path);
  if (!metadata.isFile() || metadata.size === 0) {
    throw new Error(`Required production resource is missing or empty: ${relativePath}`);
  }
  return {
    path: relativePath,
    bytes: metadata.size,
    sha256: sha256(path),
  };
}

function packagedLocaleFiles(app) {
  const directory = join(app, "Contents", "Resources", "_up_", "src", "locales");
  return readdirSync(directory, { withFileTypes: true }).map((entry) => {
    if (!entry.isFile()) {
      throw new Error(`Production locale resource is not a regular file: ${entry.name}`);
    }
    return entry.name;
  });
}

function relativePath(root, path) {
  return relative(root, path).split(sep).join("/");
}

export async function run({ root, appPath }) {
  const app = resolve(appPath);
  const config = JSON.parse(readFileSync(resolve(root, "src-tauri", "tauri.conf.json"), "utf8"));
  const configuredLocales = validateConfiguredLocaleResources(config);
  const locales = packagedLocaleFiles(app);
  if (JSON.stringify(sorted(locales)) !== JSON.stringify(sorted(SUPPORTED_LOCALE_FILES))) {
    throw new Error("Signed application does not contain exactly the 12 supported locale catalogs.");
  }

  const testSources = filesUnder(app)
    .map((path) => relativePath(app, path))
    .filter((path) =>
      /(?:^|\/)(?:__tests__|fixtures|mock_data|test_data)(?:\/|$)/iu.test(path)
      || /(?:^|[._-])(?:test|spec)(?:[._-]|$)/iu.test(path),
    );
  if (testSources.length > 0) {
    throw new Error(`Test source or fixture data is present in the application: ${testSources[0]}`);
  }

  const resources = REQUIRED_RESOURCE_FILES.map((path) => requireNonemptyFile(app, path));
  for (const locale of SUPPORTED_LOCALE_FILES) {
    resources.push(requireNonemptyFile(
      app,
      `Contents/Resources/_up_/src/locales/${locale}`,
    ));
  }

  return {
    passed: true,
    evidence: {
      schema_version: 1,
      configured_locale_resources: configuredLocales,
      packaged_locale_files: sorted(locales),
      required_resources: resources,
      test_sources: [],
    },
  };
}
