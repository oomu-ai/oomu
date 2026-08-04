import { createHash } from "node:crypto";
import {
  lstatSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  realpathSync,
} from "node:fs";
import { homedir } from "node:os";
import { join, relative, resolve, sep } from "node:path";

export const name = "build_path_privacy";

const MAX_FILE_BYTES = 512 * 1024 * 1024;
const MAX_BUNDLE_BYTES = 2 * 1024 * 1024 * 1024;
const MARKER_POLICY = "oomu.macos-local-build-paths.v1";
const MACH_O_MAGICS = new Set([
  "feedface", "cefaedfe", "feedfacf", "cffaedfe",
  "cafebabe", "bebafeca", "cafebabf", "bfbafeca",
]);

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function shownPath(root, path) {
  return relative(root, path).split(sep).join("/");
}

function isMachO(bytes) {
  return bytes.length >= 4 && MACH_O_MAGICS.has(bytes.subarray(0, 4).toString("hex"));
}

function markers(repositoryRoot, homeDirectory) {
  const exactRepository = resolve(repositoryRoot);
  const exactHome = resolve(homeDirectory);
  return [
    { rule: "absolute_macos_user_path", value: "/Users/" },
    { rule: "absolute_macos_data_user_path", value: "/System/Volumes/Data/Users/" },
    { rule: "exact_repository_build_path", value: exactRepository },
    { rule: "exact_builder_home_path", value: exactHome },
  ]
    .filter(({ value }, index, values) =>
      value !== "/" && values.findIndex((entry) => entry.value === value) === index)
    .map((entry) => ({ ...entry, bytes: Buffer.from(entry.value, "utf8") }));
}

function markerFindings(bytes, path, kind, pathMarkers) {
  const findings = [];
  for (const marker of pathMarkers) {
    let count = 0;
    let cursor = 0;
    while ((cursor = bytes.indexOf(marker.bytes, cursor)) >= 0) {
      count += 1;
      cursor += marker.bytes.length;
    }
    if (count > 0) findings.push({ path, kind, rule: marker.rule, count });
  }
  return findings;
}

export function inspectBuildPathPrivacy(
  appPath,
  {
    repositoryRoot = resolve(import.meta.dirname, "..", ".."),
    homeDirectory = homedir(),
  } = {},
) {
  const app = realpathSync(resolve(appPath));
  const pathMarkers = markers(repositoryRoot, homeDirectory);
  const findings = [];
  let filesScanned = 0;
  let machOFilesScanned = 0;
  let bytesScanned = 0;
  let symlinksInspected = 0;

  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      const metadata = lstatSync(path);
      const relativePath = shownPath(app, path);
      if (metadata.isSymbolicLink()) {
        symlinksInspected += 1;
        findings.push(...markerFindings(
          Buffer.from(readlinkSync(path), "utf8"), relativePath, "symlink", pathMarkers,
        ));
      } else if (metadata.isDirectory()) {
        visit(path);
      } else if (metadata.isFile()) {
        filesScanned += 1;
        if (metadata.size > MAX_FILE_BYTES || bytesScanned + metadata.size > MAX_BUNDLE_BYTES) {
          findings.push({
            path: relativePath,
            kind: "file",
            rule: "path_scan_bound_exceeded",
            count: 1,
          });
          continue;
        }
        const bytes = readFileSync(path);
        bytesScanned += bytes.length;
        const kind = isMachO(bytes) ? "mach_o" : "bundle_file";
        if (kind === "mach_o") machOFilesScanned += 1;
        findings.push(...markerFindings(bytes, relativePath, kind, pathMarkers));
      }
    }
  };
  visit(app);
  return {
    filesScanned,
    machOFilesScanned,
    bytesScanned,
    symlinksInspected,
    findings,
    markerPolicySha256: sha256(Buffer.from(MARKER_POLICY, "utf8")),
  };
}

export async function run({ root, appPath }) {
  const inspection = inspectBuildPathPrivacy(appPath, { repositoryRoot: root });
  if (inspection.findings.length > 0) {
    throw new Error(
      `Packaged application contains ${inspection.findings.length} local build-path finding(s).`,
    );
  }
  return {
    passed: true,
    evidence: {
      schema_version: 1,
      recursive: true,
      files_scanned: inspection.filesScanned,
      mach_o_files_scanned: inspection.machOFilesScanned,
      bytes_scanned: inspection.bytesScanned,
      symlinks_inspected: inspection.symlinksInspected,
      marker_policy_sha256: inspection.markerPolicySha256,
      findings: [],
    },
  };
}
