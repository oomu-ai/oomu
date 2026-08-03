// CI guard: every locale must be valid JSON and is compared to en-US.json.
//
// English-first policy (2026-07-01): development adds keys to en-US only and
// the runtime falls back to English, so MISSING keys in other locales are
// reported as warnings, not failures — they'll be batch-translated later.
// What still FAILS the check is a *bad* entry: a German stub copied into a
// non-German locale, or a long English string pasted in as a "translation"
// (an honest omission falls back cleanly; a pasted fallback hides the gap
// from the translation tooling).
// Run with: node scripts/check-locale-keys.mjs
//
// The pure helpers and the stub pattern are exported so a unit test can lock
// them: this guard was previously faked twice (German stubs, then a regex that
// matched a literal "\b" instead of a word boundary and so never fired). The
// test asserts the pattern actually catches a German stub.
import { readFileSync, readdirSync, realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const localesDir = join(dirname(fileURLToPath(import.meta.url)), "..", "src", "locales");
const REFERENCE = "en-US.json";

export function leafPaths(obj, prefix = "") {
  const paths = [];
  for (const [key, value] of Object.entries(obj)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (value && typeof value === "object" && !Array.isArray(value)) {
      paths.push(...leafPaths(value, path));
    } else {
      paths.push(path);
    }
  }
  return paths;
}

export function valueAtPath(obj, path) {
  return path.split(".").reduce((node, key) => {
    if (!node || typeof node !== "object") return undefined;
    return node[key];
  }, obj);
}

// Word boundaries must be real (`\b`), not the escaped `\\b` that shipped before
// and matched a literal backslash, never a stub. The regression test locks this.
export const GERMAN_WORKFLOW_STUB_PATTERN =
  /\b(Wird erstellt|Dieser Workflow|Der Workflow|Prüfe|Gespeichert|Vorlage|Kompilierte|Genehmigung|Ausführung|Laufzeit|Zweig|Schritt|Verbinden|Bereit|Andernfalls|Speichern)\b/i;

export const EXACT_ENGLISH_ALLOWLIST = [
  "Apple Mail",
  "Mac",
  "Mail",
  "Markdown",
  "Mods",
  "OOMU",
];

export function isSuspiciousEnglishFallback(value, referenceValue) {
  if (typeof value !== "string" || typeof referenceValue !== "string") {
    return false;
  }
  const normalized = value.trim();
  if (normalized !== referenceValue.trim()) {
    return false;
  }
  if (normalized.length < 18 || !/[A-Za-z]/.test(normalized)) {
    return false;
  }
  return !EXACT_ENGLISH_ALLOWLIST.some((allowed) => normalized === allowed);
}

// Detect the Workflows-namespace stubs the audit history keeps reintroducing:
// German text copied into a non-German locale, or a long English string left
// identical to the reference (an untranslated fallback). Returns the offending
// leaf paths for `data`.
export function workflowStubPaths(data, reference, file) {
  return leafPaths(data.workflows ?? {}, "workflows").filter((path) => {
    const value = valueAtPath(data, path);
    const referenceValue = valueAtPath(reference, path);
    if (typeof value !== "string") return false;
    if (file !== "de-DE.json" && GERMAN_WORKFLOW_STUB_PATTERN.test(value)) {
      return true;
    }
    return isSuspiciousEnglishFallback(value, referenceValue);
  });
}

// Pure per-locale evaluation so the policy split (warn vs fail) is testable.
export function evaluateLocale(data, reference, file) {
  const referencePaths = leafPaths(reference);
  const referenceSet = new Set(referencePaths);
  const paths = new Set(leafPaths(data));
  return {
    missing: referencePaths.filter((path) => !paths.has(path)),
    extra: [...paths].filter((path) => !referenceSet.has(path)),
    stubs: workflowStubPaths(data, reference, file),
  };
}

// English-first policy: only bad entries block; gaps fall back to English.
export function blockingIssueCount(evaluation) {
  return evaluation.stubs.length;
}

export function runLocaleCheck() {
  const reference = JSON.parse(readFileSync(join(localesDir, REFERENCE), "utf8"));

  const locales = readdirSync(localesDir).filter(
    (file) => file.endsWith(".json") && file !== REFERENCE,
  );

  let failureCount = 0;
  for (const file of locales) {
    const data = JSON.parse(readFileSync(join(localesDir, file), "utf8"));
    const { missing, extra, stubs } = evaluateLocale(data, reference, file);
    failureCount += blockingIssueCount({ missing, extra, stubs });

    if (missing.length > 0) {
      console.warn(
        `⚠ ${file} is missing ${missing.length} key(s) (falls back to English; batch-translate later):`,
      );
      for (const path of missing) console.warn(`    ${path}`);
    }
    if (extra.length > 0) {
      console.warn(`⚠ ${file} has ${extra.length} key(s) not in ${REFERENCE}:`);
      for (const path of extra) console.warn(`    ${path}`);
    }
    if (stubs.length > 0) {
      console.error(`✗ ${file} has suspicious Workflows localization stub(s):`);
      for (const path of stubs) console.error(`    ${path}`);
    }
    if (missing.length === 0 && extra.length === 0 && stubs.length === 0) {
      console.log(`✓ ${file}`);
    }
  }

  if (failureCount > 0) {
    console.error(`\nLocale check failed with ${failureCount} blocking issue(s).`);
  } else {
    console.log("\nLocale files are valid (missing keys, if any, fall back to English).");
  }
  return failureCount;
}

// Run the check only when invoked as a CLI, so importing the helpers above for
// tests does not trigger the file scan or process.exit.
function isInvokedDirectly() {
  if (!process.argv[1]) return false;
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
}

if (isInvokedDirectly()) {
  process.exit(runLocaleCheck() > 0 ? 1 : 0);
}
