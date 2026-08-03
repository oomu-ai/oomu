import path from "node:path";
import ts from "typescript";

export function typeScriptImportSpecifiers(file, source) {
  const scriptKind = file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS;
  const sourceFile = ts.createSourceFile(
    file,
    source,
    ts.ScriptTarget.Latest,
    true,
    scriptKind,
  );
  const specifiers = new Set();
  const visit = (node) => {
    if (
      (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
      node.moduleSpecifier &&
      ts.isStringLiteralLike(node.moduleSpecifier)
    ) {
      specifiers.add(node.moduleSpecifier.text);
    }
    if (
      ts.isCallExpression(node) &&
      node.arguments.length === 1 &&
      ts.isStringLiteralLike(node.arguments[0]) &&
      (
        node.expression.kind === ts.SyntaxKind.ImportKeyword ||
        (ts.isIdentifier(node.expression) && node.expression.text === "require")
      )
    ) {
      specifiers.add(node.arguments[0].text);
    }
    ts.forEachChild(node, visit);
  };
  visit(sourceFile);
  return [...specifiers];
}

function rustWithoutTrivia(source) {
  const output = source.split("");
  let index = 0;
  let blockDepth = 0;
  const blank = (position) => {
    if (output[position] !== "\n") output[position] = " ";
  };
  while (index < source.length) {
    if (blockDepth > 0) {
      if (source.startsWith("/*", index)) {
        blank(index++);
        blank(index++);
        blockDepth += 1;
      } else if (source.startsWith("*/", index)) {
        blank(index++);
        blank(index++);
        blockDepth -= 1;
      } else {
        blank(index++);
      }
      continue;
    }
    if (source.startsWith("//", index)) {
      while (index < source.length && source[index] !== "\n") blank(index++);
      continue;
    }
    if (source.startsWith("/*", index)) {
      blank(index++);
      blank(index++);
      blockDepth = 1;
      continue;
    }
    const raw = source.slice(index).match(/^r(#+)?"/);
    if (raw) {
      const hashes = raw[1] ?? "";
      const terminator = `"${hashes}`;
      const end = source.indexOf(terminator, index + raw[0].length);
      const stop = end < 0 ? source.length : end + terminator.length;
      while (index < stop) blank(index++);
      continue;
    }
    if (source[index] === '"') {
      blank(index++);
      while (index < source.length) {
        const character = source[index];
        blank(index++);
        if (character === "\\" && index < source.length) blank(index++);
        else if (character === '"') break;
      }
      continue;
    }
    index += 1;
  }
  return output.join("");
}

export function rustCrateDependencies(source) {
  const dependencies = new Set();
  for (const match of rustWithoutTrivia(source).matchAll(
    /\bcrate::([a-zA-Z_][a-zA-Z0-9_]*)/g,
  )) {
    dependencies.add(match[1]);
  }
  return [...dependencies];
}

export function relativeModule(root, file) {
  return path.relative(root, file).replace(/\\/g, "/");
}
