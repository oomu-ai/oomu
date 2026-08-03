import { spawnSync } from "node:child_process";
import { lstatSync, readFileSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { basename, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

export const name = "oauth_secret_absence";
const MAX_SCAN_FILE_BYTES = 512 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES = 10_000;
const MAX_ARCHIVE_ENTRY_BYTES = 128 * 1024 * 1024;
const MAX_ARCHIVE_EXPANDED_BYTES = 512 * 1024 * 1024;
const FORBIDDEN_TEXT = [
  "OOMU_GOOGLE_OAUTH_CLIENT_SECRET",
  "OOMU_SLACK_OAUTH_CLIENT_SECRET",
  '"client_secret"',
  '"clientSecret"',
];

function isGoogleDesktopProtocolCredential(value) {
  // Google requires this field for the reviewed Desktop client while documenting
  // installed apps as unable to keep it confidential. Only the main executable may carry it.
  return /^GOCSPX-[A-Za-z0-9._-]{8,504}$/u.test(value);
}

function isGoogleDesktopProtocolCredentialLocation(shownPath) {
  return shownPath.replaceAll("\\", "/") === "Contents/MacOS/oomu";
}

function filesUnder(root) {
  const files = [];
  function visit(path) {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const child = resolve(path, entry.name);
      const metadata = lstatSync(child);
      if (metadata.isSymbolicLink()) continue;
      if (metadata.isDirectory()) visit(child);
      else if (metadata.isFile()) files.push({ path: child, size: metadata.size });
    }
  }
  visit(root);
  return files;
}

function configuredCanaries() {
  const encoded = process.env.OOMU_OAUTH_SECRET_SCAN_CANARIES_BASE64?.trim();
  if (!encoded) return [];
  let decoded;
  try {
    decoded = JSON.parse(Buffer.from(encoded, "base64").toString("utf8"));
  } catch {
    throw new Error("OAuth secret scan canaries are malformed.");
  }
  if (
    !Array.isArray(decoded) ||
    decoded.length === 0 ||
    decoded.length > 16 ||
    decoded.some((value) => typeof value !== "string" || value.length < 16 || value.length > 2048)
  ) {
    throw new Error("OAuth secret scan canaries are malformed.");
  }
  return decoded;
}

function forbiddenFileName(path) {
  const name = basename(path).toLowerCase();
  return (
    name === ".env" ||
    name.startsWith(".env.") ||
    /client[_-]?secret.*\.json$/.test(name) ||
    /oauth.*credentials.*\.json$/.test(name)
  );
}

function recordBufferFindings(bytes, shownPath, canaries, findings) {
  for (const marker of FORBIDDEN_TEXT) {
    if (bytes.includes(Buffer.from(marker))) {
      findings.push({ path: shownPath, rule: "forbidden_oauth_private_marker" });
    }
  }
  for (const canary of canaries) {
    if (
      isGoogleDesktopProtocolCredential(canary)
      && isGoogleDesktopProtocolCredentialLocation(shownPath)
    ) continue;
    if (bytes.includes(Buffer.from(canary))) {
      findings.push({ path: shownPath, rule: "oauth_secret_canary_detected" });
    }
  }
}

function archiveCommand(path) {
  const lower = path.toLowerCase();
  if ([".zip", ".jar", ".whl", ".ipa"].some((suffix) => lower.endsWith(suffix))) {
    return {
      list: ["/usr/bin/unzip", ["-Z1", path]],
      read: (entry) => ["/usr/bin/unzip", ["-p", path, entry]],
    };
  }
  if ([".tar", ".tar.gz", ".tgz", ".tar.bz2", ".tbz2", ".tar.xz", ".txz"].some((suffix) => lower.endsWith(suffix))) {
    return {
      list: ["/usr/bin/tar", ["-tf", path]],
      read: (entry) => ["/usr/bin/tar", ["-xOf", path, entry]],
    };
  }
  return null;
}

function inspectArchive(path, shownPath, canaries, findings) {
  const command = archiveCommand(path);
  if (!command) return { entries: 0, bytes: 0 };
  const listed = spawnSync(command.list[0], command.list[1], {
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  if (listed.status !== 0) {
    findings.push({ path: shownPath, rule: "archive_entry_scan_failed" });
    return { entries: 0, bytes: 0 };
  }
  const entries = listed.stdout.split(/\r?\n/).filter(Boolean);
  if (entries.length > MAX_ARCHIVE_ENTRIES) {
    findings.push({ path: shownPath, rule: "archive_exceeds_bounded_secret_scan" });
    return { entries: entries.length, bytes: 0 };
  }
  let expandedBytes = 0;
  for (const entry of entries) {
    const entryPath = `${shownPath}!/${entry}`;
    if (entry.endsWith("/")) continue;
    if (forbiddenFileName(entry)) {
      findings.push({ path: entryPath, rule: "forbidden_private_credential_filename" });
    }
    const read = command.read(entry);
    const extracted = spawnSync(read[0], read[1], {
      encoding: null,
      maxBuffer: MAX_ARCHIVE_ENTRY_BYTES,
    });
    if (extracted.status !== 0 || !Buffer.isBuffer(extracted.stdout)) {
      findings.push({ path: entryPath, rule: "archive_entry_scan_failed" });
      continue;
    }
    expandedBytes += extracted.stdout.length;
    if (expandedBytes > MAX_ARCHIVE_EXPANDED_BYTES) {
      findings.push({ path: shownPath, rule: "archive_exceeds_bounded_secret_scan" });
      break;
    }
    recordBufferFindings(extracted.stdout, entryPath, canaries, findings);
  }
  return { entries: entries.length, bytes: expandedBytes };
}

export function inspectSecretAbsence(root, canaries = configuredCanaries()) {
  const canonical = realpathSync(resolve(root));
  const findings = [];
  let bytesScanned = 0;
  let archiveEntriesScanned = 0;
  const files = filesUnder(canonical);
  for (const file of files) {
    const shownPath = relative(canonical, file.path);
    if (forbiddenFileName(file.path)) {
      findings.push({ path: shownPath, rule: "forbidden_private_credential_filename" });
    }
    if (file.size > MAX_SCAN_FILE_BYTES) {
      findings.push({ path: shownPath, rule: "file_exceeds_bounded_secret_scan" });
      continue;
    }
    const bytes = readFileSync(file.path);
    bytesScanned += bytes.length;
    recordBufferFindings(bytes, shownPath, canaries, findings);
    const archive = inspectArchive(file.path, shownPath, canaries, findings);
    archiveEntriesScanned += archive.entries;
    bytesScanned += archive.bytes;
    const fileProbe = spawnSync("/usr/bin/file", ["-b", file.path], { encoding: "utf8" });
    if (fileProbe.status === 0 && fileProbe.stdout.includes("Mach-O")) {
      const strings = spawnSync("/usr/bin/strings", ["-a", file.path], {
        encoding: "utf8",
        maxBuffer: 128 * 1024 * 1024,
      });
      if (strings.status !== 0) {
        findings.push({ path: shownPath, rule: "macho_string_scan_failed" });
      } else {
        recordBufferFindings(Buffer.from(strings.stdout), shownPath, canaries, findings);
      }
    }
  }
  return { files, bytesScanned, archiveEntriesScanned, findings };
}

export async function run({ appPath }) {
  const inspection = inspectSecretAbsence(appPath);
  if (inspection.findings.length > 0) {
    throw new Error(
      `Confidential OAuth material was found in ${inspection.findings.length} packaged location(s).`,
    );
  }
  return {
    passed: true,
    evidence: {
      schema_version: 1,
      recursive: true,
      files_scanned: inspection.files.length,
      bytes_scanned: inspection.bytesScanned,
      archive_entries_scanned: inspection.archiveEntriesScanned,
      canaries_checked: configuredCanaries().length,
      findings: [],
    },
  };
}

function parseArgs(argv) {
  const appIndex = argv.indexOf("--app");
  const outputIndex = argv.indexOf("--output");
  if (appIndex < 0 || !argv[appIndex + 1]) {
    throw new Error("Usage: oauth-secret-absence.mjs --app <OOMU.app> [--output report.json]");
  }
  return { app: argv[appIndex + 1], output: outputIndex >= 0 ? argv[outputIndex + 1] : null };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    const args = parseArgs(process.argv.slice(2));
    const result = await run({ appPath: args.app });
    const encoded = `${JSON.stringify(result, null, 2)}\n`;
    if (args.output) writeFileSync(resolve(args.output), encoded, { mode: 0o600 });
    process.stdout.write(encoded);
  } catch (error) {
    console.error(`OAUTH SECRET ABSENCE GATE FAILED: ${error.message}`);
    process.exit(1);
  }
}
