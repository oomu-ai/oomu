#!/usr/bin/env node

import { readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const root = path.resolve(import.meta.dirname, "..");
const baselinePath = process.env.OOMU_UNUSED_EXPORT_BASELINE
  ? path.resolve(process.env.OOMU_UNUSED_EXPORT_BASELINE)
  : path.join(root, "scripts", "unused-export-baseline.json");
const frameworkExports = new Set([
  "dynamic",
  "generateMetadata",
  "generateStaticParams",
  "metadata",
  "preferredRegion",
  "revalidate",
  "runtime",
  "viewport",
]);

function relativeSource(analysisRoot, sourceFile) {
  return path.relative(analysisRoot, sourceFile.fileName).replace(/\\/g, "/");
}

function isTestSource(relative) {
  return (
    relative.includes("/__tests__/") ||
    /\.(?:test|spec)\.[cm]?[jt]sx?$/.test(relative)
  );
}

function isRuntimeSource(analysisRoot, sourceFile) {
  const relative = relativeSource(analysisRoot, sourceFile);
  return (
    relative.startsWith("src/") &&
    !isTestSource(relative) &&
    !relative.endsWith(".d.ts")
  );
}

function canonicalSymbol(checker, symbol) {
  if (!symbol) return null;
  const visited = new Set();
  let current = symbol;
  while (current.flags & ts.SymbolFlags.Alias) {
    if (visited.has(current)) return current;
    visited.add(current);
    const resolved = checker.getAliasedSymbol(current);
    if (!resolved || resolved === current) return current;
    current = resolved;
  }
  return current;
}

function moduleSourceFile(checker, moduleSpecifier) {
  const symbol = checker.getSymbolAtLocation(moduleSpecifier);
  return canonicalSymbol(checker, symbol)?.declarations?.find(ts.isSourceFile) ?? null;
}

function importedSources(checker, sourceFile) {
  const sources = new Set();
  const visit = (node) => {
    if (
      (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
      node.moduleSpecifier &&
      ts.isStringLiteralLike(node.moduleSpecifier)
    ) {
      const target = moduleSourceFile(checker, node.moduleSpecifier);
      if (target) sources.add(target);
    }
    if (
      ts.isCallExpression(node) &&
      node.expression.kind === ts.SyntaxKind.ImportKeyword &&
      node.arguments.length === 1 &&
      ts.isStringLiteralLike(node.arguments[0])
    ) {
      const target = moduleSourceFile(checker, node.arguments[0]);
      if (target) sources.add(target);
    }
    ts.forEachChild(node, visit);
  };
  visit(sourceFile);
  return sources;
}

function isProductionEntry(relative) {
  return (
    /^src\/app\/(?:.*\/)?(?:layout|page|route)\.[cm]?[jt]sx?$/.test(relative) ||
    /^src\/(?:instrumentation|middleware)\.[cm]?[jt]s$/.test(relative)
  );
}

function reachableProductionSources(program, checker, analysisRoot) {
  const runtime = new Set(program.getSourceFiles().filter(
    (sourceFile) => isRuntimeSource(analysisRoot, sourceFile),
  ));
  const reachable = new Set();
  const queue = [...runtime].filter((sourceFile) =>
    isProductionEntry(relativeSource(analysisRoot, sourceFile))
  );
  while (queue.length > 0) {
    const sourceFile = queue.shift();
    if (!sourceFile || reachable.has(sourceFile)) continue;
    reachable.add(sourceFile);
    for (const target of importedSources(checker, sourceFile)) {
      if (runtime.has(target) && !reachable.has(target)) queue.push(target);
    }
  }
  return reachable;
}

function isDeclarationIdentifier(node) {
  const parent = node.parent;
  if (!parent) return false;
  if (
    ts.isImportClause(parent) ||
    ts.isImportSpecifier(parent) ||
    ts.isNamespaceImport(parent) ||
    ts.isExportSpecifier(parent)
  ) {
    return true;
  }
  return "name" in parent && parent.name === node && (
    ts.isVariableDeclaration(parent) ||
    ts.isFunctionDeclaration(parent) ||
    ts.isFunctionExpression(parent) ||
    ts.isClassDeclaration(parent) ||
    ts.isClassExpression(parent) ||
    ts.isInterfaceDeclaration(parent) ||
    ts.isTypeAliasDeclaration(parent) ||
    ts.isEnumDeclaration(parent) ||
    ts.isMethodDeclaration(parent) ||
    ts.isPropertyDeclaration(parent) ||
    ts.isParameter(parent) ||
    ts.isPropertyAssignment(parent)
  );
}

function isTypeOnlyReference(node) {
  let current = node.parent;
  while (current && !ts.isStatement(current) && !ts.isSourceFile(current)) {
    if (ts.isTypeNode(current) && !ts.isExpressionWithTypeArguments(current)) return true;
    current = current.parent;
  }
  return false;
}

function referencedSymbols(checker, sourceFiles) {
  const symbols = new Set();
  for (const sourceFile of sourceFiles) {
    const visit = (node) => {
      if (
        ts.isIdentifier(node) &&
        !isDeclarationIdentifier(node) &&
        !isTypeOnlyReference(node)
      ) {
        const referenced = ts.isShorthandPropertyAssignment(node.parent)
          ? checker.getShorthandAssignmentValueSymbol(node.parent)
          : checker.getSymbolAtLocation(node);
        const symbol = canonicalSymbol(checker, referenced);
        if (symbol) symbols.add(symbol);
      }
      ts.forEachChild(node, visit);
    };
    visit(sourceFile);
  }
  return symbols;
}

function frameworkOwned(relative, exportName) {
  return (
    /(?:^|\/)(?:layout|page|route)\.[cm]?[jt]sx?$/.test(relative) &&
    (exportName === "default" || frameworkExports.has(exportName))
  );
}

export function analyzeProductionExports(program, analysisRoot) {
  const checker = program.getTypeChecker();
  const runtimeSources = program.getSourceFiles().filter(
    (sourceFile) => isRuntimeSource(analysisRoot, sourceFile),
  );
  const reachable = reachableProductionSources(program, checker, analysisRoot);
  const runtimeReferences = referencedSymbols(checker, reachable);
  const testReferences = referencedSymbols(
    checker,
    program.getSourceFiles().filter((sourceFile) =>
      isTestSource(relativeSource(analysisRoot, sourceFile))
    ),
  );
  const unusedExports = [];
  const testOnlyExports = [];

  for (const sourceFile of runtimeSources) {
    const moduleSymbol = checker.getSymbolAtLocation(sourceFile);
    if (!moduleSymbol) continue;
    const relative = relativeSource(analysisRoot, sourceFile);
    for (const exported of checker.getExportsOfModule(moduleSymbol)) {
      const exportName = exported.escapedName.toString();
      const target = canonicalSymbol(checker, exported);
      if (!target || frameworkOwned(relative, exportName)) continue;
      // TypeScript erases pure type declarations. They are compile-time
      // contracts, not production JavaScript surface, so a runtime dead-export
      // gate must neither grandfather nor ask callers to delete them.
      if (!(target.flags & ts.SymbolFlags.Value)) continue;
      const key = `${relative}#${exportName}`;
      if (runtimeReferences.has(target)) continue;
      if (testReferences.has(target)) {
        testOnlyExports.push(key);
        continue;
      }
      unusedExports.push(key);
    }
  }
  unusedExports.sort();
  testOnlyExports.sort();
  return { unusedExports, testOnlyExports };
}

function loadProgram() {
  const configPath = path.join(root, "tsconfig.json");
  const configFile = ts.readConfigFile(configPath, ts.sys.readFile);
  if (configFile.error) {
    throw new Error(ts.flattenDiagnosticMessageText(configFile.error.messageText, "\n"));
  }
  const parsed = ts.parseJsonConfigFileContent(configFile.config, ts.sys, root);
  return ts.createProgram(parsed.fileNames, parsed.options);
}

function compareBaseline(label, currentValues, approvedValues) {
  const approved = new Set(approvedValues ?? []);
  const current = new Set(currentValues);
  return {
    introduced: currentValues.filter((candidate) => !approved.has(candidate)),
    stale: [...approved].filter((candidate) => !current.has(candidate)),
    label,
  };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.includes("--fix")) {
    console.error("unused-export-ratchet: automatic source deletion is disabled; review symbol reachability before removing an export");
    process.exit(1);
  }
  const analysis = analyzeProductionExports(loadProgram(), root);
  if (process.argv.includes("--print-baseline")) {
    console.log(JSON.stringify(analysis, null, 2));
    process.exit(0);
  }

  const baseline = JSON.parse(readFileSync(baselinePath, "utf8"));
  const comparisons = [
    compareBaseline("UNUSED_EXPORT", analysis.unusedExports, baseline.unusedExports),
  ];
  const hasChanges = comparisons.some(({ introduced, stale }) => introduced.length || stale.length);
  if (hasChanges) {
    console.error("unused-export-ratchet: FAIL");
    for (const comparison of comparisons) {
      for (const candidate of comparison.introduced) {
        console.error(`  - NEW_${comparison.label} ${candidate}`);
      }
      for (const candidate of comparison.stale) {
        console.error(`  - STALE_${comparison.label}_BASELINE ${candidate}`);
      }
    }
    process.exit(1);
  }
  console.log(`unused-export-ratchet: PASS (${analysis.unusedExports.length} reviewed production candidates; ${analysis.testOnlyExports.length} test-only candidates reported separately)`);
}
