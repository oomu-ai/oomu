// CI guard: user-facing chrome must route text through `t()`, never hardcoded
// JSX literals — otherwise the batch-translation tooling (which reads the locale
// files) cannot see the string and it ships English-only forever.
//
// English-first policy (2026-07-01) means we develop in en-US keys; this guard
// is the other half of that deal. Every TSX component below the two product
// component roots is governed by default. Only explicit user-content,
// developer-only, and test surfaces are denied, and every denial records why.
//
// Heuristic, deliberately conservative to avoid false positives:
//   - bare JSX text nodes that are Capitalized words: >Save Profile<
//   - English literal UI attributes: title/placeholder/aria-label="Save"
//   - brace-wrapped JSX strings: {"Save Profile"}
//   - English strings in local objects whose properties render directly in JSX
// Run with: node scripts/check-jsx-literals.mjs
import { existsSync, readFileSync, readdirSync, realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, relative, resolve, sep } from "node:path";
import ts from "typescript";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

export const SCAN_ROOTS = ["src/app/components", "src/components"];

// Exact product exemptions. Do not add an entry merely to make the gate pass.
export const DENYLIST = Object.freeze({
  "src/components/AgentProfileView.tsx":
    "user-entered content editor (product decision 2026-07-01)",
  "src/app/components/DeveloperPanel.tsx": "developer-only surface",
  "src/app/components/ActivityPane.tsx": "developer-only surface",
  "src/app/components/dashboard/OperationsControl.tsx":
    "developer-only (mounted from DeveloperPanel only)",
});

// Pattern exemptions are reason-bearing too, so no class of skipped files is
// invisible during review.
export const DENYLIST_PATTERNS = Object.freeze([
  Object.freeze({
    pattern: /(?:^|\/)__tests__(?:\/|$)/,
    reason: "test-only fixture or harness under __tests__",
  }),
  Object.freeze({
    pattern: /\.test\.tsx$/,
    reason: "test-only TSX file",
  }),
]);

// Compatibility name retained for callers that still import the old exemption
// object while the gate migrates from allow-by-file to govern-by-default.
export const EXEMPT_NOTE = DENYLIST;

function toRepoPath(value) {
  return value.split(sep).join("/");
}

export function denialReason(rel) {
  const normalized = toRepoPath(rel);
  if (DENYLIST[normalized]) return DENYLIST[normalized];
  return DENYLIST_PATTERNS.find(({ pattern }) => pattern.test(normalized))?.reason ?? null;
}

function walkTsxFiles(directory, root, files) {
  if (!existsSync(directory)) return;
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const absolute = join(directory, entry.name);
    if (entry.isDirectory()) {
      walkTsxFiles(absolute, root, files);
    } else if (entry.isFile() && entry.name.endsWith(".tsx")) {
      const rel = toRepoPath(relative(root, absolute));
      if (!denialReason(rel)) files.push(rel);
    }
  }
}

export function discoverScannedFiles(root = repoRoot) {
  const absoluteRoot = resolve(root);
  const files = [];
  for (const scanRoot of SCAN_ROOTS) {
    walkTsxFiles(join(absoluteRoot, scanRoot), absoluteRoot, files);
  }
  return files.sort((left, right) => left.localeCompare(right));
}

// Compatibility export: unlike the old hand-maintained allowlist, this is the
// discovered inventory at module load. Tests that create files dynamically
// should call discoverScannedFiles(root) or runJsxLiteralCheck({ root }).
export const ENFORCED_FILES = discoverScannedFiles();

const COPY_ATTRIBUTES = new Set(["title", "placeholder", "aria-label"]);
const SVG_ELEMENTS = new Set([
  "svg",
  "path",
  "circle",
  "rect",
  "line",
  "polyline",
  "polygon",
  "g",
]);

function normalizedCopy(value) {
  return value.replace(/\s+/g, " ").trim();
}

function looksLikeEnglishCopy(value) {
  const text = normalizedCopy(value);
  return text.length >= 3 && /^[A-Z][A-Za-z]/.test(text);
}

function renderedExpressionCopy(node) {
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
    return node.text;
  }
  if (ts.isTemplateExpression(node)) {
    return [node.head.text, ...node.templateSpans.map((span) => `{value}${span.literal.text}`)]
      .join("");
  }
  return null;
}

function lineNumber(sourceFile, node) {
  return sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1;
}

function unwrapExpression(node) {
  let current = node;
  while (
    ts.isParenthesizedExpression(current) ||
    ts.isAsExpression(current) ||
    ts.isSatisfiesExpression(current) ||
    ts.isTypeAssertionExpression(current) ||
    ts.isNonNullExpression(current)
  ) {
    current = current.expression;
  }
  return current;
}

function openingElement(node) {
  if (ts.isJsxElement(node)) return node.openingElement;
  if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) return node;
  return null;
}

function hasAttribute(opening, name, sourceFile) {
  return opening.attributes.properties.some(
    (property) =>
      ts.isJsxAttribute(property) && property.name.getText(sourceFile) === name,
  );
}

function isIgnorableJsxNode(node, sourceFile) {
  let current = node.parent;
  while (current && !ts.isSourceFile(current)) {
    const opening = openingElement(current);
    if (opening) {
      const tagName = opening.tagName.getText(sourceFile).toLowerCase();
      if (SVG_ELEMENTS.has(tagName) || hasAttribute(opening, "aria-hidden", sourceFile)) {
        return true;
      }
    }
    current = current.parent;
  }
  return false;
}

function jsxExpressionAncestor(node) {
  let current = node.parent;
  while (current) {
    if (ts.isJsxExpression(current)) return current;
    if (
      ts.isSourceFile(current) ||
      ts.isFunctionLike(current) ||
      ts.isClassLike(current)
    ) {
      return null;
    }
    current = current.parent;
  }
  return null;
}

function expressionAndRenderedObjectFindings(source, findings) {
  const sourceFile = ts.createSourceFile(
    "component.tsx",
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  const localObjects = new Map();
  const renderedProperties = new Map();

  function collect(node) {
    const unwrappedInitializer =
      ts.isVariableDeclaration(node) && node.initializer
        ? unwrapExpression(node.initializer)
        : null;
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      unwrappedInitializer &&
      ts.isObjectLiteralExpression(unwrappedInitializer)
    ) {
      const properties = new Map();
      for (const property of unwrappedInitializer.properties) {
        const initializer =
          ts.isPropertyAssignment(property) && property.initializer
            ? unwrapExpression(property.initializer)
            : null;
        if (
          ts.isPropertyAssignment(property) &&
          (ts.isIdentifier(property.name) ||
            ts.isStringLiteral(property.name) ||
            ts.isNumericLiteral(property.name)) &&
          initializer &&
          (ts.isStringLiteral(initializer) ||
            ts.isNoSubstitutionTemplateLiteral(initializer)) &&
          looksLikeEnglishCopy(initializer.text)
        ) {
          properties.set(property.name.text, initializer);
        }
      }
      if (properties.size > 0) localObjects.set(node.name.text, properties);
    }

    if (
      ts.isJsxText(node) &&
      looksLikeEnglishCopy(node.text) &&
      !isIgnorableJsxNode(node, sourceFile)
    ) {
      findings.push({
        line: lineNumber(sourceFile, node),
        kind: "text",
        text: normalizedCopy(node.text),
      });
    }

    if (
      ts.isJsxAttribute(node) &&
      COPY_ATTRIBUTES.has(node.name.getText(sourceFile)) &&
      node.initializer &&
      ts.isStringLiteral(node.initializer) &&
      looksLikeEnglishCopy(node.initializer.text) &&
      !isIgnorableJsxNode(node, sourceFile)
    ) {
      findings.push({
        line: lineNumber(sourceFile, node),
        kind: "attr",
        text: `${node.name.getText(sourceFile)}="${normalizedCopy(node.initializer.text)}"`,
      });
    }

    const expressionCopy = renderedExpressionCopy(node);
    if (
      expressionCopy &&
      jsxExpressionAncestor(node) &&
      looksLikeEnglishCopy(expressionCopy) &&
      !isIgnorableJsxNode(node, sourceFile)
    ) {
      findings.push({
        line: lineNumber(sourceFile, node),
        kind: "expression",
        text: normalizedCopy(expressionCopy),
      });
    }

    if (
      ts.isPropertyAccessExpression(node) &&
      ts.isIdentifier(node.expression) &&
      jsxExpressionAncestor(node) &&
      !isIgnorableJsxNode(node, sourceFile)
    ) {
      const objectName = node.expression.text;
      const propertyName = node.name.text;
      if (!renderedProperties.has(objectName)) renderedProperties.set(objectName, new Set());
      renderedProperties.get(objectName).add(propertyName);
    }
    ts.forEachChild(node, collect);
  }
  collect(sourceFile);

  for (const [objectName, propertyNames] of renderedProperties) {
    const properties = localObjects.get(objectName);
    if (!properties) continue;
    for (const propertyName of propertyNames) {
      const initializer = properties.get(propertyName);
      if (!initializer) continue;
      findings.push({
        line: lineNumber(sourceFile, initializer),
        kind: "copy-object",
        text: `${objectName}.${propertyName}: ${normalizedCopy(initializer.text)}`,
      });
    }
  }
}

export function findLiteralsInSource(source) {
  const findings = [];
  expressionAndRenderedObjectFindings(source, findings);
  return findings.sort((left, right) => left.line - right.line || left.kind.localeCompare(right.kind));
}

export function runJsxLiteralCheck(options = {}) {
  const root = typeof options === "string" ? options : (options.root ?? repoRoot);
  const files = typeof options === "object" && options.files
    ? options.files
    : discoverScannedFiles(root);
  let failureCount = 0;
  for (const rel of files) {
    let source;
    try {
      source = readFileSync(join(root, rel), "utf8");
    } catch {
      console.warn(`⚠ ${rel} (discovered but no longer present)`);
      continue;
    }
    const findings = findLiteralsInSource(source);
    if (findings.length === 0) {
      console.log(`✓ ${rel}`);
      continue;
    }
    failureCount += findings.length;
    console.error(`✗ ${rel} has ${findings.length} hardcoded literal(s):`);
    for (const finding of findings) {
      console.error(`    ${rel}:${finding.line}  [${finding.kind}]  ${finding.text}`);
    }
  }

  if (failureCount > 0) {
    console.error(`\nJSX literal check failed with ${failureCount} issue(s). Route the text through t().`);
  } else {
    console.log("\nAll discovered product surfaces route user copy through t().");
  }
  return failureCount;
}

function isInvokedDirectly() {
  if (!process.argv[1]) return false;
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
}

if (isInvokedDirectly()) {
  process.exit(runJsxLiteralCheck() > 0 ? 1 : 0);
}
