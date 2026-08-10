import type { ChatTurnContext } from "@/lib/chatTurnContext";
import type { ChatAttachment } from "./attachments";
import { approvedLocalFileAttachments, approvedLocalFilesPrompt, directLocalFileReadPaths, unescapeShellPath } from "./directLocalFileRead";
import { candidateLocalPathsFromText } from "./localPathIntent";
import { shouldDeferFileShortcutForRoutine } from "./chatRoutineHandoff";

type DirectLocalCommand =
  | { kind: "read"; path: string }
  | { kind: "read_many"; paths: string[] }
  | { kind: "list"; path: string }
  | { kind: "write"; path: string; content: string }
  | { kind: "delete"; path: string }
  | { kind: "shell"; command: string };

const directListCommandPatterns = [
  /\b(?:ls|dir)\b/i,
  /\b(?:list|show)\b.{0,32}\b(?:files?|directories|directory|folders?|path|workspace|repo|repository|sandbox|downloads?|desktop|documents)\b/i,
];
const explicitTerminalListContextPattern = /\b(?:terminal|shell|command[ -]line|cli)\b/i;
const explicitTerminalListExecutionPattern = /\b(?:run|execute|use|using)\b/i;
const explicitTerminalListNegationPattern =
  /\b(?:do\s+not|don't|dont|never|without)\b.{0,48}\b(?:run|execute|use|using)\b/i;
const standardUserTerminalListPaths = new Set(["~/Downloads", "~/Documents", "~/Desktop"]);
const directWriteCommandPattern =
  /\b(?:create|make|write|save|touch)\b.{0,80}\b(?:file|markdown|md|document|note)\b|\b(?:echo|printf)\b[\s\S]+?>/i;
const directWriteMixedIntentPattern =
  /\b(?:read|review|inspect|analy[sz]e|research|retrieve|search|browse|look\s+up|calendar|e-?mail|mail|send|deliver|schedule|event|attach|verify|validate|read\s+back|web\s+source|public\s+source|official\s+source)\b/i;
const directWriteSequencePattern =
  /\b(?:and\s+then|then|after(?:wards?)?|before|finally|next)\b|[;\n]/i;
const explicitEmptyWritePattern =
  /\b(?:empty|blank|zero[- ]byte)\s+(?:file|markdown|document|note)\b|\b(?:file|markdown|document|note)\s+(?:empty|blank)\b|\b(?:content|contents|text|body)\s*(?:is|as|to|of|with|inside|containing)?\s*(?:""|'')/i;
const directDeleteWholeTurnPattern =
  /^\s*(?:please\s+)?(?:(?:run|execute)\s+(?:the\s+)?(?:shell\s+)?command\s*:?\s*)?(?:delete|remove|trash|unlink|rm)\b\s+(?:"[^"\n]+"|'[^'\n]+'|[^\n;,!?]+?)\s*[.!]?\s*$/i;
const directDeleteCompoundIntentPattern =
  /(?:^|[\s,;])(?:and\s+then|then|after(?:wards?)?|before|finally|next|but|instead)(?=[\s,:;]|$)|[;\n]|\band\s+(?:read|review|inspect|analy[sz]e|research|retrieve|search|browse|create|make|write|save|copy|move|rename|send|deliver|schedule|open|run|execute|verify|validate)\b/i;
const quotedCommandPathPattern = /"([^"\n]+)"|'([^'\n]+)'/;
const quotedTextPattern = /"([^"\n]*)"|'([^'\n]*)'/g;
const fileLikeNamePattern =
  /[^/\\\n]+\.(?:md|markdown|txt|json|csv|tsv|log|html|css|js|jsx|ts|tsx|py|rs|toml|yaml|yml)$/i;
const deletePathTokenPattern =
  /\b(?:delete|remove|trash|unlink|rm)\b(?:\s+(?:the|this|that|a))?(?:\s+(?:local|sandbox|workspace))?(?:\s+(?:file|path))?(?:\s+(?:named|at|from))?\s+([~./A-Za-z0-9][^\s,;]*)/i;
const listPathTokenPattern =
  /\b(?:ls|dir|list|show)\b(?:\s+(?:the|this|that|a))?(?:\s+(?:files?|directories|directory|folders?|contents|path))?(?:\s+(?:in|of|at|from))?\s+([~./A-Za-z0-9][^\s,;]*)/i;
const rootListTargetPattern =
  /\b(?:current|this)\s+(?:directory|folder|workspace|repo|repository)\b|\b(?:workspace|repo|repository|sandbox)\s+(?:root|directory|folder)?\b/i;
const shellRedirectionWritePattern =
  /^\s*(?:(?:run|execute)\s+(?:the\s+)?(?:shell\s+)?command\s*:?\s*)?(?:echo|printf)\s+(?:"([^"\n]*)"|'([^'\n]*)'|([^\n>]+?))\s*>\s*(?:"([^"\n]+)"|'([^'\n]+)'|([^\s\n]+))\s*[.!]?\s*$/i;
const touchWritePattern =
  /^\s*(?:(?:run|execute)\s+(?:the\s+)?(?:shell\s+)?command\s*:?\s*)?touch\s+(?:"([^"\n]+)"|'([^'\n]+)'|([^\s\n]+))\s*[.!]?\s*$/i;
const namedFilePattern =
  /\b(?:called|named|as)\s+(?:"([^"\n]+\.[A-Za-z0-9]+)"|'([^'\n]+\.[A-Za-z0-9]+)'|([^"'\n]+?\.[A-Za-z0-9]+))(?=\s+(?:with|containing|that|in|inside|under|at|to|and)\b|[.?!,]|$)/i;
const contentPhrasePatterns = [
  /\b(?:content|contents|text|body)\s*(?:is|as|to|of|with|inside|containing)?\s*(?:"([^"\n]*)"|'([^'\n]*)')/i,
  /\b(?:with|containing|contains|that says|saying)\s+(?:the\s+)?(?:words?\s+)?(?:"([^"\n]*)"|'([^'\n]*)')/i,
  /\b(?:with|containing|contains|that says|saying)\s+(?:the\s+)?(?:words?\s+)?([^\.\n]+)$/i,
] as const;
const directShellCommandPatterns = [
  /^\s*(?:run|execute)\s+(?:the\s+)?(?:shell\s+)?command\s*:?\s+([\s\S]+)$/i,
  /^\s*(?:run|execute)\s+`([^`]+)`\s*$/i,
  /^\s*(?:run|execute)\s+((?:npm|pnpm|yarn|bun|cargo|git|python3?|node|npx|make|pytest|go|rustc|pwd|date|cat|mkdir|touch|cp|mv|rm|echo|printf|find|grep|rg|sed|awk|sh|bash|zsh|open|osascript)\b[\s\S]*)$/i,
];

export function detectDirectLocalCommand(text: string): DirectLocalCommand | null {
  const normalized = text.trim();
  if (!normalized) return null;
  if (shouldDeferFileShortcutForRoutine(normalized)) return null;

  const writeCommand = detectDirectWriteCommand(normalized);
  if (writeCommand) return writeCommand;

  const deleteCommand = detectDirectDeleteCommand(normalized);
  if (deleteCommand) return deleteCommand;
  const explicitTerminalList = detectExplicitTerminalListCommand(normalized);
  if (explicitTerminalList) return explicitTerminalList;
  if (directListCommandPatterns.some((pattern) => pattern.test(normalized))) {
    const path = directLocalCommandPath(normalized, "list");
    return { kind: "list", path: path ?? "" };
  }
  const readPaths = directLocalFileReadPaths(
    normalized,
    candidateLocalPathsFromText(normalized),
  );
  if (readPaths?.length === 1) return { kind: "read", path: readPaths[0] };
  if (readPaths && readPaths.length > 1) return { kind: "read_many", paths: readPaths };

  const shellCommand = detectDirectShellCommand(normalized);
  return shellCommand ? { kind: "shell", command: shellCommand } : null;
}

export function directLocalReadPathsForCommand(command: DirectLocalCommand | null) {
  if (command?.kind === "read") return [command.path];
  return command?.kind === "read_many" ? command.paths : [];
}

export async function prepareDirectLocalReadTurn(
  command: DirectLocalCommand,
  message: string,
  turn: ChatTurnContext,
  existing: ChatAttachment[],
) {
  const paths = directLocalReadPathsForCommand(command);
  return {
    attachments: await approvedLocalFileAttachments(paths, message, turn, existing),
    modelMessage: approvedLocalFilesPrompt(message, paths),
  };
}

function detectExplicitTerminalListCommand(text: string): DirectLocalCommand | null {
  if (
    !explicitTerminalListContextPattern.test(text) ||
    !explicitTerminalListExecutionPattern.test(text) ||
    explicitTerminalListNegationPattern.test(text) ||
    !/\bls\b/i.test(text)
  ) {
    return null;
  }
  const paths = candidateLocalPathsFromText(text).filter((path) =>
    standardUserTerminalListPaths.has(path)
  );
  return paths.length === 1 ? { kind: "shell", command: `ls ${paths[0]}` } : null;
}

function detectDirectDeleteCommand(text: string): DirectLocalCommand | null {
  // Destructive shortcuts are intentionally narrower than conversational
  // intent detection: the whole accepted turn must be one affirmative delete.
  // Negated or multi-action prose continues through the planner instead.
  if (
    !directDeleteWholeTurnPattern.test(text) ||
    directDeleteCompoundIntentPattern.test(text)
  ) {
    return null;
  }
  const path = directLocalCommandPath(text, "delete");
  return path === null ? null : { kind: "delete", path };
}

function detectDirectWriteCommand(text: string): DirectLocalCommand | null {
  const redirected = shellRedirectionWritePattern.exec(text);
  if (redirected) {
    const content = (redirected[1] ?? redirected[2] ?? redirected[3] ?? "").trim();
    const path = (redirected[4] ?? redirected[5] ?? redirected[6] ?? "").trim();
    if (path) return { kind: "write", path, content };
  }

  const touched = touchWritePattern.exec(text);
  if (touched) {
    const path = (touched[1] ?? touched[2] ?? touched[3] ?? "").trim();
    return path ? { kind: "write", path, content: "" } : null;
  }
  if (!directWriteCommandPattern.test(text)) return null;

  const textWithoutQuotedContent = text.replace(quotedTextPattern, '""');
  if (
    directWriteMixedIntentPattern.test(textWithoutQuotedContent) ||
    directWriteSequencePattern.test(textWithoutQuotedContent)
  ) {
    return null;
  }

  const localPaths = candidateLocalPathsFromText(text);
  const explicitPaths = localPaths.filter((path) =>
    fileLikeNamePattern.test(unescapeShellPath(path)),
  );
  const uniqueExplicitPaths = [
    ...new Set(explicitPaths.map((path) => unescapeShellPath(path))),
  ];
  if (uniqueExplicitPaths.length > 1) return null;
  const explicitPath = uniqueExplicitPaths[0];
  const directoryPaths = localPaths.filter(
    (path) => !fileLikeNamePattern.test(unescapeShellPath(path)),
  );
  if (directoryPaths.length > 1) return null;

  const namedFile = namedFileFromText(text);
  const quotedFile = quotedSegments(text).find((segment) =>
    fileLikeNamePattern.test(segment),
  );
  const namedTarget = namedFile ?? quotedFile;
  if (
    explicitPath &&
    namedTarget &&
    unescapeShellPath(explicitPath) !== unescapeShellPath(namedTarget) &&
    !unescapeShellPath(explicitPath).endsWith(`/${unescapeShellPath(namedTarget)}`)
  ) {
    return null;
  }

  const path =
    explicitPath ?? joinLocalPath(directoryPaths[0] ?? "", namedTarget ?? "");
  if (!path) return null;
  const content = contentFromWriteCommand(text, namedTarget ?? explicitPath);
  if (content === undefined && !explicitEmptyWritePattern.test(text)) return null;
  return { kind: "write", path, content: content ?? "" };
}

function detectDirectShellCommand(text: string) {
  for (const pattern of directShellCommandPatterns) {
    const command = pattern.exec(text)?.[1]?.trim();
    if (command) return cleanShellCommand(command);
  }
  return null;
}

function namedFileFromText(text: string) {
  const match = namedFilePattern.exec(text);
  return (match?.[1] ?? match?.[2] ?? match?.[3] ?? "")
    .trim()
    .replace(/^(?:a|an|the)\s+/i, "")
    .replace(/[.:!?)\]}]+$/g, "");
}

function contentFromWriteCommand(text: string, pathHint?: string) {
  for (const pattern of contentPhrasePatterns) {
    const match = pattern.exec(text);
    const content = (match?.[1] ?? match?.[2] ?? "").trim();
    if (content && content !== pathHint) return content;
  }
  return quotedSegments(text).find(
    (segment) => segment !== pathHint && !fileLikeNamePattern.test(segment),
  );
}

function quotedSegments(text: string) {
  return [...text.matchAll(quotedTextPattern)]
    .map((match) => (match[1] ?? match[2] ?? "").trim())
    .filter(Boolean);
}

function joinLocalPath(directory: string, fileName: string) {
  const cleanedFileName = fileName.trim().replace(/[.:!?)\]}]+$/g, "");
  if (!cleanedFileName) return "";
  if (isHostLocalPath(cleanedFileName) || cleanedFileName.startsWith("./")) {
    return cleanedFileName;
  }
  const cleanedDirectory = directory.trim().replace(/\/+$/g, "");
  return cleanedDirectory ? `${cleanedDirectory}/${cleanedFileName}` : cleanedFileName;
}

function cleanShellCommand(command: string) {
  return command
    .replace(/[.?!]\s*$/g, "")
    .replace(
      /\s+\bin\s+(?:the\s+)?(?:current\s+)?(?:workspace|repo|repository|directory)\s*$/i,
      "",
    )
    .trim();
}

function directLocalCommandPath(text: string, kind: "list" | "delete") {
  const localPath = candidateLocalPathsFromText(text)[0];
  if (localPath) return localPath;

  const quoted = quotedCommandPathPattern.exec(text);
  const quotedPath = (quoted?.[1] ?? quoted?.[2] ?? "").trim();
  if (quotedPath) return quotedPath;

  const tokenPattern = kind === "delete" ? deletePathTokenPattern : listPathTokenPattern;
  const tokenPath = tokenPattern.exec(text)?.[1]?.trim().replace(/[.:!?)\]}]+$/g, "");
  if (
    tokenPath &&
    tokenPath !== "-" &&
    !/^(?:file|folder|directory|path)$/i.test(tokenPath)
  ) {
    return tokenPath;
  }
  if (kind === "list" && rootListTargetPattern.test(text)) return "";
  if (
    kind === "list" &&
    /^(?:run|execute)?\s*(?:the\s+)?(?:command\s+)?(?:ls|dir)\s*$/i.test(text)
  ) {
    return "";
  }
  return null;
}

export function isHostLocalPath(path: string) {
  return path.startsWith("/") || path === "~" || path.startsWith("~/");
}
