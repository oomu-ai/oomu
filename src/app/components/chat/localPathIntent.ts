type LocalPathReferenceKind =
  | "absolute"
  | "home_relative"
  | "workspace_relative"
  | "file_uri";

type LocalPathReference = {
  rawText: string;
  normalizedText: string;
  sourceSpan: { start: number; end: number };
  kind: LocalPathReferenceKind;
  markdownWrapped: boolean;
};

type WrappedCandidate = {
  rawText: string;
  start: number;
  end: number;
  markdownWrapped: boolean;
};

const fileUriPattern = /file:\/\/[^\s<>`"']+/gi;
const markdownCodePattern = /`([^`\n]+)`/g;
const quotedPathPattern = /"((?:\\.|[^"\\\n])*)"|'((?:\\.|[^'\\\n])*)'/g;
const anglePathPattern = /<([^<>\n]+)>/g;
const barePathPattern = /(?:^|[\s([{,:;])((?:~\/|\/(?!\/)|\.\.?\/|[A-Za-z0-9_.-]+\/)(?:\\.|[^\s,;<>`"'\])}])+)/g;
const bareAbsolutePathStartPattern = /(?:^|[\s([{,:;])(\/(?:Applications|Library|System|Users|Volumes|bin|dev|etc|home|opt|private|sbin|tmp|usr|var)(?:\/|$))/g;
const knownUnquotedFileExtensionPattern = /\.(?:avif|bmp|c|cc|cpp|css|csv|doc|docx|gif|go|heic|heif|htm|html|jpeg|jpg|js|json|jsx|log|md|markdown|mjs|mov|mp3|mp4|numbers|pages|parquet|pdf|png|ppt|pptx|py|rs|rtf|svg|swift|toml|ts|tsv|tsx|txt|webp|xls|xlsx|xml|yaml|yml)(?=$|[.,:;!?\])}]|\s+(?:and|or|then|from|with|without|to|in|into|inside|under|at|for|before|after|while|but|please|analy[sz]e|archive|attach|compare|copy|create|delete|describe|draft|email|explain|export|import|inspect|list|move|open|prepare|publish|read|recommend|remove|rename|review|run|save|send|share|show|summari[sz]e|trash|upload|write)\b)/gi;
const unquotedPathHardTerminatorPattern = /[\n\r,;:!?\])}<>`"']/;
const unquotedDirectoryClausePattern = /\s+(?:and\s+then|then|and|or|but)\s+(?=(?:analy[sz]e|archive|attach|compare|copy|create|delete|describe|draft|email|explain|export|import|inspect|list|move|open|prepare|publish|read|recommend|remove|rename|review|run|save|send|share|show|summari[sz]e|trash|upload|write)\b)/i;
const unquotedCourtesyBoundaryPattern = /\s+please\b/i;
const unquotedNextAbsolutePathPattern = /\s+(?:(?:and|or|then)\s+)?(?=\/(?:Applications|Library|System|Users|Volumes|bin|dev|etc|home|opt|private|sbin|tmp|usr|var)(?:\/|$))/;
const ambiguousUnquotedPathTailPattern = /(?:\s+(?:because|for|from|in|on|with|without)\s+[^/]+|\s+(?:carefully|contents|now|today|yesterday))\s*$/i;
const markdownLinkDestinationPattern = /\]\(\s*<?([^\s)>]+)>?(?:\s+["'][^"']*["'])?\s*\)/g;
const knownAbsolutePathRoots = new Set([
  "Applications", "Library", "System", "Users", "Volumes", "bin", "dev", "etc",
  "home", "opt", "private", "sbin", "tmp", "usr", "var",
]);
const standardUserFolderCandidates = [
  {
    path: "~/Downloads",
    pattern: /\b(?:(?:my|the|this|local|user(?:'s)?)\s+)?downloads?\s+(?:folder|directory)\b|\b(?:my|the|this|local|user(?:'s)?)\s+downloads?\b|\b(?:list|show|open|read|inspect|view|ls|tree|cat|run|execute)\b.{0,64}\bdownloads?\b/i,
  },
  {
    path: "~/Documents",
    pattern: /\b(?:(?:my|the|this|local|user(?:'s)?)\s+)?documents\s+(?:folder|directory)\b|\b(?:my|the|this|local|user(?:'s)?)\s+documents\b|\b(?:list|show|open|read|inspect|view|ls|tree|cat|run|execute)\b.{0,64}\bdocuments\b/i,
  },
  {
    path: "~/Desktop",
    pattern: /\b(?:(?:my|the|this|local|user(?:'s)?)\s+)?desktop\s+(?:folder|directory)\b|\b(?:my|the|this|local|user(?:'s)?)\s+desktop\b|\b(?:list|show|open|read|inspect|view|ls|tree|cat|run|execute)\b.{0,64}\bdesktop\b/i,
  },
] as const;

export function parseLocalPathReferences(text: string): LocalPathReference[] {
  const candidates = [
    ...wrappedCandidates(text, markdownCodePattern, true),
    ...wrappedCandidates(text, quotedPathPattern, false),
    ...wrappedCandidates(text, anglePathPattern, false),
    ...bareCandidates(text),
  ];
  const references: LocalPathReference[] = [];
  for (const candidate of candidates.sort((left, right) => left.start - right.start)) {
    const reference = normalizeCandidate(text, candidate);
    if (!reference || references.some((entry) => spansOverlap(entry.sourceSpan, reference.sourceSpan))) {
      continue;
    }
    references.push(reference);
  }
  return references.slice(0, 12);
}

export function candidateLocalPathsFromText(text: string) {
  const references = parseLocalPathReferences(text);
  const paths = references.map((reference) => reference.normalizedText);
  const textOutsideParsedPaths = maskSourceSpans(
    text,
    references.map((reference) => reference.sourceSpan),
  );
  for (const candidate of standardUserFolderCandidates) {
    if (candidate.pattern.test(textOutsideParsedPaths) && !paths.includes(candidate.path)) {
      paths.push(candidate.path);
    }
  }
  return [...new Set(paths)].slice(0, 5);
}

export function unescapeShellPath(path: string) {
  return path.replace(/\\([\s\S])/g, "$1");
}

export function localPathReferenceVariants(text: string, path: string) {
  return parseLocalPathReferences(text)
    .filter((reference) => reference.normalizedText === path)
    .map((reference) => reference.rawText);
}

export function localPathReferenceIndex(text: string, path: string) {
  const indexes = parseLocalPathReferences(text)
    .filter((reference) => reference.normalizedText === path)
    .map((reference) => reference.sourceSpan.start);
  return indexes.length > 0 ? Math.min(...indexes) : -1;
}

function wrappedCandidates(text: string, pattern: RegExp, markdownWrapped: boolean) {
  const candidates: WrappedCandidate[] = [];
  for (const match of text.matchAll(pattern)) {
    const rawText = match[1] ?? match[2] ?? "";
    const offset = match[0].indexOf(rawText);
    candidates.push({
      rawText,
      start: (match.index ?? 0) + Math.max(offset, 0),
      end: (match.index ?? 0) + Math.max(offset, 0) + rawText.length,
      markdownWrapped,
    });
  }
  return candidates;
}

function bareCandidates(text: string) {
  const candidates: WrappedCandidate[] = [];
  for (const match of text.matchAll(fileUriPattern)) {
    candidates.push({ rawText: match[0], start: match.index ?? 0, end: (match.index ?? 0) + match[0].length, markdownWrapped: false });
  }
  candidates.push(...bareAbsolutePathsWithSpaces(text));
  for (const match of text.matchAll(barePathPattern)) {
    const rawText = match[1] ?? "";
    if (rawText.startsWith("/")) continue;
    const start = (match.index ?? 0) + match[0].lastIndexOf(rawText);
    candidates.push({ rawText, start, end: start + rawText.length, markdownWrapped: false });
  }
  return candidates;
}

function bareAbsolutePathsWithSpaces(text: string) {
  const candidates: WrappedCandidate[] = [];
  for (const match of text.matchAll(bareAbsolutePathStartPattern)) {
    const root = match[1] ?? "";
    const start = (match.index ?? 0) + match[0].lastIndexOf(root);
    const remainder = text.slice(start);
    const hardTerminator = unquotedPathHardTerminatorPattern.exec(remainder);
    const bounded = hardTerminator
      ? remainder.slice(0, hardTerminator.index)
      : remainder;
    const boundaryIndices = [
      unquotedDirectoryClausePattern.exec(bounded)?.index ?? -1,
      unquotedCourtesyBoundaryPattern.exec(bounded)?.index ?? -1,
      unquotedNextAbsolutePathPattern.exec(bounded)?.index ?? -1,
    ].filter((index) => index >= 0);
    const proseBoundary = boundaryIndices.length > 0 ? Math.min(...boundaryIndices) : -1;
    const clauseBounded = proseBoundary >= 0 ? bounded.slice(0, proseBoundary) : bounded;
    const fileEnd = unquotedFilePathEnd(clauseBounded);
    const directoryBoundary = fileEnd === null
      ? unquotedDirectoryPathEnd(clauseBounded)
      : null;
    const end = fileEnd ?? directoryBoundary;
    if (end === null) continue;
    const rawText = clauseBounded.slice(0, end).trimEnd();
    if (fileEnd === null && ambiguousUnquotedPathTailPattern.test(rawText)) continue;
    candidates.push({
      rawText,
      start,
      end: start + rawText.length,
      markdownWrapped: false,
    });
  }
  return candidates;
}

function unquotedFilePathEnd(value: string) {
  const matches = [...value.matchAll(knownUnquotedFileExtensionPattern)]
    .sort((left, right) => (left.index ?? 0) - (right.index ?? 0));
  const match = matches[0];
  return match ? (match.index ?? 0) + match[0].length : null;
}

function unquotedDirectoryPathEnd(value: string) {
  const sentenceEnd = /[.](?=\s|$)/.exec(value)?.index ?? -1;
  const clauseEnd = unquotedDirectoryClausePattern.exec(value)?.index ?? -1;
  const boundaries = [sentenceEnd, clauseEnd].filter((index) => index >= 0);
  const end = boundaries.length > 0 ? Math.min(...boundaries) : value.length;
  return value.slice(0, end).trimEnd() ? end : null;
}

function maskSourceSpans(text: string, spans: Array<{ start: number; end: number }>) {
  if (spans.length === 0) return text;
  const characters = text.split("");
  for (const span of spans) {
    for (let index = Math.max(0, span.start); index < Math.min(characters.length, span.end); index += 1) {
      characters[index] = " ";
    }
  }
  return characters.join("");
}

function normalizeCandidate(text: string, candidate: WrappedCandidate): LocalPathReference | null {
  if (insideMarkdownLinkDestination(text, candidate)) return null;
  const rawText = trimTerminalPunctuation(candidate.rawText.trim());
  if (!rawText || quotedNonActionExample(text, candidate.start)) return null;
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(rawText) && !rawText.toLowerCase().startsWith("file://")) return null;
  if (rawText.toLowerCase().startsWith("file://")) return localFileUriReference(rawText, candidate);
  const normalizedText = normalizeExtractedLocalPath(unescapeShellPath(rawText));
  const kind = localPathKind(normalizedText);
  if (!kind || !isValidLocalPathCandidate(normalizedText, kind)) return null;
  return {
    rawText,
    normalizedText,
    sourceSpan: { start: candidate.start, end: candidate.start + rawText.length },
    kind,
    markdownWrapped: candidate.markdownWrapped,
  };
}

function localFileUriReference(rawText: string, candidate: WrappedCandidate): LocalPathReference | null {
  try {
    const uri = new URL(rawText);
    if (uri.protocol !== "file:" || (uri.hostname && uri.hostname !== "localhost")) return null;
    const normalizedText = decodeURIComponent(uri.pathname);
    if (!normalizedText.startsWith("/") || normalizedText.startsWith("//")) return null;
    return { rawText, normalizedText, sourceSpan: { start: candidate.start, end: candidate.start + rawText.length }, kind: "file_uri", markdownWrapped: candidate.markdownWrapped };
  } catch {
    return null;
  }
}

function localPathKind(path: string): LocalPathReferenceKind | null {
  if (path.startsWith("~/")) return "home_relative";
  if (path.startsWith("/") && !path.startsWith("//")) return "absolute";
  if (path.startsWith("./") || path.startsWith("../") || /^[A-Za-z0-9_.-]+\//.test(path)) return "workspace_relative";
  return null;
}

function isValidLocalPathCandidate(path: string, kind: LocalPathReferenceKind) {
  if (path.length <= 1 || /[<>\n\r]/.test(path)) return false;
  if (kind === "absolute") {
    const root = path.slice(1).split("/", 1)[0];
    if (!path.includes("/", 1) && !knownAbsolutePathRoots.has(root)) return false;
    if (isSingleSegmentSlashCommandCandidate(path)) return false;
  }
  return path.split("/").some((segment) => segment.length > 0 && segment !== "." && segment !== "..");
}

function normalizeExtractedLocalPath(path: string) {
  return path.startsWith("///") ? `/${path.slice(3)}` : path;
}

function trimTerminalPunctuation(path: string) {
  return path.replace(/[,:;!?]+$/g, "").replace(/[.)\]}]+$/g, (suffix) => suffix.slice(0, Math.max(0, suffix.length - unmatchedClosingCount(path, suffix))));
}

function unmatchedClosingCount(path: string, suffix: string) {
  const pairs: Record<string, [string, string]> = { ")": ["(", ")"], "]": ["[", "]"], "}": ["{", "}"] };
  let removable = 0;
  for (const character of [...suffix].reverse()) {
    if (character === ".") { removable += 1; continue; }
    const pair = pairs[character];
    if (pair && count(path, pair[1]) > count(path, pair[0])) removable += 1;
  }
  return removable;
}

function count(value: string, needle: string) {
  return value.split(needle).length - 1;
}

function insideMarkdownLinkDestination(text: string, span: { start: number; end: number }) {
  for (const match of text.matchAll(markdownLinkDestinationPattern)) {
    const raw = match[1] ?? "";
    const start = (match.index ?? 0) + match[0].indexOf(raw);
    if (span.start >= start && span.end <= start + raw.length) return true;
  }
  return false;
}

function quotedNonActionExample(text: string, start: number) {
  const prefix = text.slice(Math.max(0, start - 48), start).replace(/[`"'<]\s*$/, "");
  return /(?:example|literal|sample|e\.g\.)\s*(?:path\s*)?(?:is|:)?\s*$/i.test(prefix);
}

function spansOverlap(left: { start: number; end: number }, right: { start: number; end: number }) {
  return left.start < right.end && right.start < left.end;
}

function isSingleSegmentSlashCommandCandidate(path: string) {
  if (!path.startsWith("/") || path.startsWith("//") || path.includes("/", 1)) return false;
  const segment = path.slice(1);
  return /^[A-Za-z][\w-]*$/.test(segment) && !knownAbsolutePathRoots.has(segment);
}
