import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const PATCHED_NEXT_ADVISORIES = Object.freeze([
  "GHSA-8h8q-6873-q5fj",
  "GHSA-26hh-7cqf-hhc6",
  "GHSA-6gpp-xcg3-4w24",
  "GHSA-m99w-x7hq-7vfj",
  "GHSA-89xv-2m56-2m9x",
  "GHSA-68g3-v927-f742",
  "GHSA-4633-3j49-mh5q",
  "GHSA-qx2v-qp2m-jg93",
  "GHSA-6g55-p6wh-862q",
  "GHSA-r28c-9q8g-f849",
  "GHSA-f88m-g3jw-g9cj",
]);

export function assertProductionAudit(audit) {
  const vulnerabilities = audit?.metadata?.vulnerabilities;
  if (!vulnerabilities || typeof vulnerabilities !== "object") {
    throw new Error("Production dependency audit did not contain vulnerability totals.");
  }
  const high = Number(vulnerabilities.high ?? 0);
  const critical = Number(vulnerabilities.critical ?? 0);
  if (!Number.isFinite(high) || !Number.isFinite(critical) || high > 0 || critical > 0) {
    throw new Error(`Production dependency audit is blocked: ${high} high, ${critical} critical.`);
  }
  return { high, critical };
}

function walk(directory) {
  const paths = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) paths.push(...walk(path));
    else if (entry.isFile()) paths.push(path);
  }
  return paths;
}

export function verifyStaticExport(exportRoot) {
  const root = resolve(exportRoot);
  const index = join(root, "index.html");
  if (!existsSync(index) || statSync(index).size === 0) {
    throw new Error("Static export is missing a non-empty index.html.");
  }
  const files = walk(root);
  const forbidden = files
    .map((path) => relative(root, path))
    .filter((path) => /(?:^|\/)(?:server|middleware|pages-manifest|routes-manifest)(?:\/|\.|$)/u.test(path));
  if (forbidden.length > 0) {
    throw new Error(`Static export contains server runtime material: ${forbidden.join(", ")}`);
  }
  for (const htmlPath of files.filter((path) => path.endsWith(".html"))) {
    const html = readFileSync(htmlPath, "utf8");
    const references = [...html.matchAll(/(?:src|href)=["']([^"']+)["']/gu)]
      .map((match) => match[1])
      .filter((value) => value.startsWith("/") && !value.startsWith("//"));
    for (const reference of references) {
      const local = reference.split(/[?#]/u)[0];
      const candidate = join(root, local.replace(/^\/+/, ""));
      if (!existsSync(candidate)) {
        throw new Error(`${relative(root, htmlPath)} references missing packaged asset ${local}.`);
      }
    }
  }
  return { fileCount: files.length, htmlCount: files.filter((path) => path.endsWith(".html")).length };
}

export function readAndAssertAudit(path) {
  return assertProductionAudit(JSON.parse(readFileSync(resolve(path), "utf8")));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [auditPath, exportRoot] = process.argv.slice(2);
  if (!auditPath || !exportRoot) {
    throw new Error("Usage: next-security-upgrade-contract.mjs <npm-audit.json> <out-directory>");
  }
  const audit = readAndAssertAudit(auditPath);
  const exported = verifyStaticExport(exportRoot);
  process.stdout.write(`${JSON.stringify({ audit, exported })}\n`);
}
