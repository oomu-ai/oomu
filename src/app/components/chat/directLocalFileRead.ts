import { invoke } from "@/lib/invoke";
import type { ChatTurnContext } from "@/lib/chatTurnContext";
import { ATTACHMENT_LIMITS } from "@/lib/attachmentProcessing";
import { hasMutatingLocalIntent } from "./executionIntentPolicy";
import type { ChatAttachment } from "./attachments";
import {
  localPathReferenceIndex,
  localPathReferenceVariants,
  unescapeShellPath,
} from "./localPathIntent";

export { unescapeShellPath } from "./localPathIntent";

const inferredStandardUserFolders = new Set(["~/Downloads", "~/Documents", "~/Desktop"]);
const indirectPathReferencePattern =
  /\breview\b.{0,32}\b(?:whether|sentence|wording|path)\b|\b(?:sentence|error|message|log)\b.{0,64}\b(?:mentions?|contains?|references?)\b|\b(?:only|just)\s+(?:an?\s+)?example\b|\b(?:example|literal)\s+path\b|\b(?:ask|have|tell)\s+(?!me\b|us\b|oomu\b)(?:the\s+)?[A-Za-z][\w'-]*\s+to\b/i;
const additionalMutationPattern =
  /\b(?:save|move|rename|copy|archive|upload|send|share|export|import|attach|post|email|transmit|run|execute|print|publish)\b/i;
const compoundFileOutcomePattern =
  /\b(?:compare|synthesize|prepare|recommend|produce|draft|create|plan|evaluate)\b/gi;
const negatedActionPrefixPattern =
  /\b(?:do\s+not|don't|dont|never|without|avoid|avoiding)\s+(?:[a-z'-]+\s+){0,4}$/i;

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

function directLocalFileReadPaths(text: string, candidates: string[]) {
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
    hasPositiveCompoundFileOutcome(intentText) ||
    indirectPathReferencePattern.test(intentText)
  ) return null;
  // Native Shield inspection classifies the exact approved target as a file
  // or directory. Filename syntax cannot do that safely on macOS.
  return explicitPaths;
}

function hasPositiveCompoundFileOutcome(text: string) {
  for (const match of text.matchAll(compoundFileOutcomePattern)) {
    const actionIndex = match.index ?? 0;
    const clausePrefix = text
      .slice(0, actionIndex)
      .split(/[.!?;\n]/)
      .at(-1) ?? "";
    if (!negatedActionPrefixPattern.test(clausePrefix)) {
      return true;
    }
  }
  return false;
}

export function directLocalFileReadPath(text: string, candidates: string[]) {
  const paths = directLocalFileReadPaths(text, candidates);
  return paths?.length === 1 ? paths[0] : null;
}

export function approvedLocalFilePrompt(message: string, path: string) {
  return localPathReferenceVariants(message, path)
    .reduce((current, candidate) => current.split(candidate).join("[approved file]"), message);
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
