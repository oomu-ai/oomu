import ts from "typescript";

export const SOURCE_LIMITS = Object.freeze({
  typescript: Object.freeze({
    lines: 1_500,
    bytes: 100_000,
    maxLineBytes: 500,
    functionLines: 120,
    complexity: 15,
  }),
  rust: Object.freeze({
    lines: 1_500,
    bytes: 100_000,
    maxLineBytes: 500,
    functionLines: 150,
    complexity: 25,
  }),
  script: Object.freeze({
    lines: 1_500,
    bytes: 100_000,
    maxLineBytes: 500,
    functionLines: 120,
    complexity: 15,
  }),
});

const METRIC_KEYS = Object.freeze([
  "lines",
  "bytes",
  "maxLineBytes",
  "functionLines",
  "complexity",
]);

export function sourceKind(relativePath) {
  if (relativePath.startsWith("src-tauri/") && relativePath.endsWith(".rs")) {
    return "rust";
  }
  if (relativePath.startsWith("scripts/")) {
    return "script";
  }
  return "typescript";
}

function physicalMetrics(source) {
  const physicalLines = source === ""
    ? []
    : source.endsWith("\n")
      ? source.slice(0, -1).split("\n")
      : source.split("\n");
  return {
    lines: physicalLines.length,
    bytes: Buffer.byteLength(source),
    maxLineBytes: physicalLines.reduce(
      (maximum, line) => Math.max(maximum, Buffer.byteLength(line.replace(/\r$/, ""))),
      0,
    ),
  };
}

function tsFunctionName(node, sourceFile) {
  if (node.name && ts.isIdentifier(node.name)) return node.name.text;
  if (ts.isVariableDeclaration(node.parent) && ts.isIdentifier(node.parent.name)) {
    return node.parent.name.text;
  }
  if (ts.isPropertyAssignment(node.parent)) {
    return node.parent.name.getText(sourceFile);
  }
  return `<anonymous@${sourceFile.getLineAndCharacterOfPosition(node.getStart()).line + 1}>`;
}

function tsFunctionComplexity(root) {
  let complexity = 1;
  const visit = (node) => {
    if (node !== root && ts.isFunctionLike(node)) return;
    if (
      ts.isIfStatement(node) ||
      ts.isForStatement(node) ||
      ts.isForInStatement(node) ||
      ts.isForOfStatement(node) ||
      ts.isWhileStatement(node) ||
      ts.isDoStatement(node) ||
      ts.isCatchClause(node) ||
      ts.isConditionalExpression(node) ||
      ts.isCaseClause(node)
    ) {
      complexity += 1;
    }
    ts.forEachChild(node, visit);
  };
  visit(root);
  return complexity;
}

function measureTypeScript(relativePath, source, scriptKind) {
  const sourceFile = ts.createSourceFile(
    relativePath,
    source,
    ts.ScriptTarget.Latest,
    true,
    scriptKind,
  );
  const functions = [];
  const visit = (node) => {
    if (ts.isFunctionLike(node) && node.body) {
      const start = sourceFile.getLineAndCharacterOfPosition(node.getStart()).line + 1;
      const end = sourceFile.getLineAndCharacterOfPosition(node.end).line + 1;
      functions.push({
        name: tsFunctionName(node, sourceFile),
        lines: end - start + 1,
        complexity: tsFunctionComplexity(node),
      });
    }
    ts.forEachChild(node, visit);
  };
  visit(sourceFile);
  return functions;
}

function blankRustTrivia(source) {
  const output = source.split("");
  let index = 0;
  let blockDepth = 0;
  const blank = (position) => {
    if (output[position] !== "\n") output[position] = " ";
  };

  while (index < source.length) {
    if (blockDepth > 0) {
      if (source.startsWith("/*", index)) {
        blank(index);
        blank(index + 1);
        blockDepth += 1;
        index += 2;
      } else if (source.startsWith("*/", index)) {
        blank(index);
        blank(index + 1);
        blockDepth -= 1;
        index += 2;
      } else {
        blank(index);
        index += 1;
      }
      continue;
    }
    if (source.startsWith("//", index)) {
      while (index < source.length && source[index] !== "\n") {
        blank(index);
        index += 1;
      }
      continue;
    }
    if (source.startsWith("/*", index)) {
      blank(index);
      blank(index + 1);
      blockDepth = 1;
      index += 2;
      continue;
    }

    const rawStop = rustRawStringStop(source, index);
    if (rawStop !== null) {
      const stop = rawStop;
      while (index < stop) {
        blank(index);
        index += 1;
      }
      continue;
    }
    const stringStop = rustQuotedStringStop(source, index);
    if (stringStop !== null) {
      while (index < stringStop) blank(index++);
      continue;
    }
    if (source[index] === "'") {
      const charLiteral = source.slice(index).match(/^'(?:\\.|[^'\\\n])'/);
      if (charLiteral) {
        for (let offset = 0; offset < charLiteral[0].length; offset += 1) {
          blank(index + offset);
        }
        index += charLiteral[0].length;
        continue;
      }
    }
    index += 1;
  }
  return output.join("");
}

function rustRawStringStop(source, index) {
  const raw = source.slice(index).match(/^r(#+)?"/);
  if (!raw) return null;
  const terminator = `"${raw[1] ?? ""}`;
  const end = source.indexOf(terminator, index + raw[0].length);
  return end < 0 ? source.length : end + terminator.length;
}

function rustQuotedStringStop(source, index) {
  if (source[index] !== '"') return null;
  let cursor = index + 1;
  while (cursor < source.length) {
    if (source[cursor] === "\\") {
      cursor += 2;
      continue;
    }
    const character = source[cursor++];
    if (character === '"') break;
  }
  return Math.min(cursor, source.length);
}

function rustTokens(source) {
  const sanitized = blankRustTrivia(source);
  const tokens = [];
  const pattern = /[A-Za-z_][A-Za-z0-9_]*|=>|&&|\|\||\?|[{}()[\];,<>:+\-*/%=!.&|]/g;
  let line = 1;
  let cursor = 0;
  for (const match of sanitized.matchAll(pattern)) {
    const position = match.index ?? 0;
    while (cursor < position) {
      if (sanitized[cursor] === "\n") line += 1;
      cursor += 1;
    }
    tokens.push({ value: match[0], line });
    cursor = position + match[0].length;
  }
  return tokens;
}

function measureRust(source) {
  const tokens = rustTokens(source);
  const functions = [];
  for (let index = 0; index < tokens.length; index += 1) {
    if (tokens[index].value !== "fn") continue;
    const name = tokens[index + 1]?.value ?? `<anonymous@${tokens[index].line}>`;
    let bodyStart = index + 1;
    while (
      bodyStart < tokens.length &&
      tokens[bodyStart].value !== "{" &&
      tokens[bodyStart].value !== ";"
    ) {
      bodyStart += 1;
    }
    if (tokens[bodyStart]?.value !== "{") continue;
    let depth = 0;
    let bodyEnd = bodyStart;
    let complexity = 1;
    for (; bodyEnd < tokens.length; bodyEnd += 1) {
      const token = tokens[bodyEnd].value;
      if (token === "{") depth += 1;
      if (token === "}") {
        depth -= 1;
        if (depth === 0) break;
      }
      if (["if", "for", "while", "loop", "match", "=>", "&&", "||", "?"].includes(token)) {
        complexity += 1;
      }
    }
    functions.push({
      name,
      lines: (tokens[bodyEnd]?.line ?? tokens[bodyStart].line) - tokens[index].line + 1,
      complexity,
    });
    index = bodyEnd;
  }
  return functions;
}

function measureShell(source) {
  const lines = source.split("\n");
  const functions = [];
  for (let index = 0; index < lines.length; index += 1) {
    const declaration = lines[index].match(/^\s*(?:function\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*\(\)\s*\{/);
    if (!declaration) continue;
    let depth = 0;
    let end = index;
    let complexity = 1;
    for (; end < lines.length; end += 1) {
      const code = lines[end].replace(/#.*/, "");
      depth += (code.match(/\{/g) ?? []).length;
      depth -= (code.match(/\}/g) ?? []).length;
      complexity += (code.match(/\b(?:if|elif|for|while|until|case)\b/g) ?? []).length;
      complexity += (code.match(/&&|\|\|/g) ?? []).length;
      if (depth === 0) break;
    }
    functions.push({ name: declaration[1], lines: end - index + 1, complexity });
    index = end;
  }
  return functions;
}

export function measureSourceFunctions(relativePath, source) {
  const kind = sourceKind(relativePath);
  if (kind === "rust") {
    return measureRust(source);
  }
  if (relativePath.endsWith(".sh")) return measureShell(source);
  const scriptKind = relativePath.endsWith(".tsx")
    ? ts.ScriptKind.TSX
    : relativePath.endsWith(".ts") || relativePath.endsWith(".mts")
      ? ts.ScriptKind.TS
      : ts.ScriptKind.JS;
  return measureTypeScript(relativePath, source, scriptKind);
}

export function measureSource(relativePath, source) {
  const kind = sourceKind(relativePath);
  const functions = measureSourceFunctions(relativePath, source);
  const longestFunction = functions.reduce(
    (maximum, candidate) => candidate.lines > maximum.lines ? candidate : maximum,
    { name: "<none>", lines: 0, complexity: 0 },
  );
  const mostComplexFunction = functions.reduce(
    (maximum, candidate) => candidate.complexity > maximum.complexity ? candidate : maximum,
    { name: "<none>", lines: 0, complexity: 0 },
  );
  return {
    kind,
    ...physicalMetrics(source),
    functionLines: longestFunction.lines,
    functionLinesName: longestFunction.name,
    complexity: mostComplexFunction.complexity,
    complexityName: mostComplexFunction.name,
  };
}

export function metricKeys() {
  return [...METRIC_KEYS];
}
