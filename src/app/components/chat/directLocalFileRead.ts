import { invoke } from "@/lib/invoke";
import type { ChatTurnContext } from "@/lib/chatTurnContext";
import { ATTACHMENT_LIMITS } from "@/lib/attachmentProcessing";
import { hasMutatingLocalIntent } from "./executionIntentPolicy";
import type { ChatAttachment } from "./attachments";
import {
  candidateLocalPathsFromText,
  localPathReferenceIndex,
  localPathReferenceVariants,
  parseLocalPathReferences,
  unescapeShellPath,
} from "./localPathIntent";

export { unescapeShellPath } from "./localPathIntent";

const inferredStandardUserFolders = new Set(["~/Downloads", "~/Documents", "~/Desktop"]);
const indirectPathReferencePattern =
  /\breview\b.{0,32}\b(?:whether|sentence|wording|path)\b|\b(?:sentence|error|message|log)\b.{0,64}\b(?:mentions?|contains?|references?)\b|\b(?:only|just)\s+(?:an?\s+)?example\b|\b(?:example|literal)\s+path\b|\b(?:ask|have|tell)\s+(?!me\b|us\b|oomu\b)(?:the\s+)?[A-Za-z][\w'-]*\s+to\b/i;
const additionalMutationPattern =
  /\b(?:save|move|rename|copy|archive|upload|send|share|export|import|attach|post|email|transmit|run|execute|print|publish)\b/i;
const generatedArtifactPattern =
  /\b(?:generate|produce|prepare|draft)\b[\s\S]{0,120}\b(?:file|document|report|pdf|docx|xlsx|pptx|spreadsheet|presentation|deck|workbook)\b/i;
const negatedFileAccessPattern =
  /\b(?:do\s+not|don't|dont|never|without)\s+(?:open|read|inspect|view|access)\b/i;
const unquotedFileNamePattern = /(?:^|[\s([{,:;])([A-Za-z0-9][A-Za-z0-9_.-]{0,179}\.(?:avif|bmp|c|cc|cpp|css|csv|doc|docx|gif|go|heic|heif|htm|html|jpeg|jpg|js|json|jsx|log|md|markdown|mjs|mov|mp3|mp4|numbers|pages|parquet|pdf|png|ppt|pptx|py|rs|rtf|svg|swift|toml|ts|tsv|tsx|txt|webp|xls|xlsx|xml|yaml|yml))(?=$|[\s,;:!?\])}])/gi;
const quotedFileNamePattern = /["'`]([^/\\\n"'`]{1,180}\.(?:avif|bmp|c|cc|cpp|css|csv|doc|docx|gif|go|heic|heif|htm|html|jpeg|jpg|js|json|jsx|log|md|markdown|mjs|mov|mp3|mp4|numbers|pages|parquet|pdf|png|ppt|pptx|py|rs|rtf|svg|swift|toml|ts|tsv|tsx|txt|webp|xls|xlsx|xml|yaml|yml))["'`]/gi;

type SignatureBlock = {
  public_key: string;
  signature: string;
  payload_hash: string;
  signed_at_ms: number;
};

type ApprovedFileReceiptToken = {
  payload: string;
  signature: SignatureBlock;
};

type PrepareApprovedChatFileResponse = {
  displayName: string;
  mimeType: string;
  byteCount: number;
  receipt: ApprovedFileReceiptToken;
};

export type NativeFileAccessResponse = {
  operation: string;
  status: string;
  message: string;
  verified: boolean;
  claims?: string[];
};

function explicitHostLocalPaths(text: string, candidates: string[]) {
  return candidates
    .map(unescapeShellPath)
    .filter((path) => localPathReferenceIndex(text, path) >= 0)
    .filter((path, index, paths) =>
      (path.startsWith("~/") || (path.startsWith("/") && !path.startsWith("//"))) &&
      paths.indexOf(path) === index,
    );
}

export function directLocalFileReadPaths(text: string, candidates: string[]) {
  const parsedExplicitPaths = explicitHostLocalPaths(
    text,
    parseLocalPathReferences(text).map((reference) => reference.normalizedText),
  );
  if (parsedExplicitPaths.length > ATTACHMENT_LIMITS.maxCount) return null;
  const specificCandidates = candidates.filter((path) => !inferredStandardUserFolders.has(path));
  const explicitPaths = explicitHostLocalPaths(text, specificCandidates);
  if (explicitPaths.length === 0) return null;
  const intentText = explicitPaths.reduce(
    (current, path) => localPathReferenceVariants(current, path)
      .reduce((withoutPath, variant) => withoutPath.split(variant).join("[local file]"), current),
    text,
  );
  if (
    hasMutatingLocalIntent(intentText) ||
    additionalMutationPattern.test(intentText) ||
    generatedArtifactPattern.test(intentText) ||
    negatedFileAccessPattern.test(intentText) ||
    indirectPathReferencePattern.test(intentText)
  ) return null;
  // A user may naturally name a file and its folder separately. Resolve that
  // one unambiguous pair before native inspection; native Shield still proves
  // the target type, identity, and permission.
  if (explicitPaths.length !== 1) return explicitPaths;
  const fileNames = standaloneFileNames(text).filter(
    (fileName) => !explicitPaths[0].endsWith(`/${fileName}`),
  );
  if (fileNames.length === 0) return explicitPaths;
  const root = explicitPaths[0].replace(/\/+$/g, "");
  const resolvedPaths = fileNames.map((fileName) => `${root}/${fileName}`);
  return resolvedPaths.length <= ATTACHMENT_LIMITS.maxCount ? resolvedPaths : null;
}

function standaloneFileNames(text: string) {
  const matches = [
    ...[...text.matchAll(quotedFileNamePattern)].map((match) => match[1]),
    ...[...text.matchAll(unquotedFileNamePattern)].map((match) => match[1]),
  ].filter((value): value is string => Boolean(value));
  return [...new Set(matches)].filter(
    (value) => value !== "." && value !== ".." && !value.includes("/") && !value.includes("\\"),
  );
}

export function directLocalFileReadPath(text: string, candidates: string[]) {
  const paths = directLocalFileReadPaths(text, candidates);
  return paths?.length === 1 ? paths[0] : null;
}

export function approvedLocalFilesPrompt(message: string, paths: string[]) {
  return paths.reduce(
    (sanitized, path) => approvedLocalFilePrompt(sanitized, path),
    message,
  );
}

export function approvedLocalFilePrompt(message: string, path: string) {
  const exactVariants = localPathReferenceVariants(message, path);
  if (exactVariants.length > 0) {
    return exactVariants.reduce(
      (current, candidate) => current.split(candidate).join("[approved file]"),
      message,
    );
  }
  const fileName = path.split("/").at(-1) ?? "";
  const withoutFolders = candidateLocalPathsFromText(message).reduce(
    (current, candidatePath) => localPathReferenceVariants(current, candidatePath)
      .reduce((sanitized, variant) => sanitized.split(variant).join("[approved folder]"), current),
    message,
  );
  return fileName
    ? withoutFolders.split(fileName).join("[approved file]")
    : withoutFolders;
}

export function approvedLocalFileContextReady(
  approvedAttachment: ChatAttachment | null,
  attachments: ChatAttachment[],
) {
  if (!approvedAttachment?.approved_file_receipt?.payload?.trim()) return false;
  return attachments.some((attachment) =>
    attachment.name === approvedAttachment.name &&
    attachment.mime_type === approvedAttachment.mime_type &&
    attachment.byte_count === approvedAttachment.byte_count &&
    attachment.approved_file_receipt?.payload === approvedAttachment.approved_file_receipt?.payload &&
    attachment.approved_file_receipt?.signature.signature ===
      approvedAttachment.approved_file_receipt?.signature.signature
  );
}

export function approvedLocalFilesContextReady(
  approvedAttachments: ChatAttachment[],
  attachments: ChatAttachment[],
) {
  if (approvedAttachments.length === 0) return false;
  const unmatched = [...attachments];
  return approvedAttachments.every((approvedAttachment) => {
    const matchIndex = unmatched.findIndex((attachment) =>
      approvedLocalFileContextReady(approvedAttachment, [attachment])
    );
    if (matchIndex < 0) return false;
    unmatched.splice(matchIndex, 1);
    return true;
  });
}

export function verifiedDirectFileReadRouteDecision(statusLabel: string) {
  return {
    route: "conversational_stream" as const,
    requires_local_access: false,
    decision_source: "verified_direct_file_read",
    reason: "The approved file is already available as bounded local context.",
    matched_signals: ["verified bounded file context"],
    status_label: statusLabel,
  };
}

function nativeDirectFileAccessRequest(
  actionKind: "file_read" | "file_list",
  path: string,
  turn: ChatTurnContext,
) {
  return {
    action: { kind: actionKind, path },
    sessionId: turn.sessionId,
    turnId: turn.turnId,
    generationToken: turn.generationToken,
  };
}

export function nativeDirectFileAccess(
  actionKind: "file_read" | "file_list",
  path: string,
  turn: ChatTurnContext,
) {
  return invoke<NativeFileAccessResponse>("execute_native_file_access", {
    request: nativeDirectFileAccessRequest(actionKind, path, turn),
  });
}

export async function approvedLocalFileAttachment(
  path: string,
  displayMessage: string,
  turn: ChatTurnContext,
  existingAttachmentCount = 0,
): Promise<ChatAttachment> {
  if (existingAttachmentCount >= ATTACHMENT_LIMITS.maxCount) {
    throw { code: "approved_file_attachment_limit" };
  }
  const response = await invoke<PrepareApprovedChatFileResponse>("prepare_approved_chat_file", {
    request: {
      access: nativeDirectFileAccessRequest("file_read", path, turn),
      displayMessage,
    },
  });
  if (
    !response.displayName.trim() ||
    !response.mimeType.trim() ||
    !Number.isSafeInteger(response.byteCount) ||
    response.byteCount <= 0 ||
    !response.receipt.payload.trim() ||
    !response.receipt.signature.signature.trim()
  ) {
    throw { code: "approved_file_unavailable" };
  }
  return {
    name: response.displayName,
    mime_type: response.mimeType,
    byte_count: response.byteCount,
    approved_file_receipt: response.receipt,
  };
}

export async function approvedLocalFileAttachments(
  paths: string[],
  displayMessage: string,
  turn: ChatTurnContext,
  existingAttachments: ChatAttachment[],
): Promise<ChatAttachment[]> {
  const uniquePaths = [...new Set(paths)];
  if (
    uniquePaths.length === 0 ||
    existingAttachments.length + uniquePaths.length > ATTACHMENT_LIMITS.maxCount
  ) {
    throw { code: "approved_file_attachment_limit" };
  }

  let decodedBytes = existingAttachments.reduce((total, attachment) => {
    if (!Number.isSafeInteger(attachment.byte_count) || attachment.byte_count < 0) {
      throw { code: "approved_file_unavailable" };
    }
    return total + attachment.byte_count;
  }, 0);
  if (decodedBytes > ATTACHMENT_LIMITS.maxDecodedBytes) {
    throw { code: "approved_file_unavailable" };
  }

  const prepared: ChatAttachment[] = [];
  for (const path of uniquePaths) {
    const attachment = await approvedLocalFileAttachment(
      path,
      displayMessage,
      turn,
      existingAttachments.length + prepared.length,
    );
    if (
      attachment.byte_count > ATTACHMENT_LIMITS.maxFileBytes ||
      decodedBytes + attachment.byte_count > ATTACHMENT_LIMITS.maxDecodedBytes
    ) {
      throw { code: "approved_file_unavailable" };
    }
    decodedBytes += attachment.byte_count;
    prepared.push(attachment);
  }
  return prepared;
}
