export type ChatAttachment = {
  name: string;
  mime_type: string;
  byte_count: number;
  data_base64?: string;
  text?: string;
  approved_file_receipt?: {
    payload: string;
    signature: {
      public_key: string;
      signature: string;
      payload_hash: string;
      signed_at_ms: number;
    };
  };
  private_data_provenance?: {
    sourceKind: "mail" | "calendar" | "contacts" | "photos" | "files" | "notes" | "reminders" | "messages" | "connector";
    sourceLabel: string;
    sourceDigest: string;
    sensitivity: "private" | "restricted";
    localTurnId: string;
    acquiredAtMs: number;
  };
};

export type VisualArtifactAnalysis = {
  image_path?: string;
  artifact_name?: string;
  mime_type?: string;
  backend?: string;
  width?: number | null;
  height?: number | null;
  page_count?: number | null;
  extracted_text?: string[];
  classifications?: string[];
  warnings?: string[];
  prompt_context?: string;
};

type PickerTextContext = {
  name: string;
  mime_type: string;
  byte_count: number;
  text: string;
  truncated: boolean;
};

const supportedVisualAttachmentMimePattern =
  /^(?:image\/(?:jpeg|jpg|png|gif|heic|heif|webp|tiff?|bmp)|application\/pdf)$/i;
const supportedVisualAttachmentExtensionPattern =
  /\.(?:jpe?g|png|gif|heic|heif|webp|pdf|tiff?|bmp)$/i;

export function mimeTypeForChatFile(file: Pick<File, "name" | "type">) {
  const declaredType = file.type?.trim();
  if (declaredType && declaredType !== "application/octet-stream") {
    return declaredType;
  }
  return (
    mimeTypeForVisualFileName(file.name) ??
    declaredType ??
    "application/octet-stream"
  );
}

function mimeTypeForVisualFileName(name: string) {
  const extension = /\.([A-Za-z0-9]+)$/.exec(name)?.[1]?.toLowerCase();
  switch (extension) {
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "png":
      return "image/png";
    case "gif":
      return "image/gif";
    case "heic":
      return "image/heic";
    case "heif":
      return "image/heif";
    case "webp":
      return "image/webp";
    case "pdf":
      return "application/pdf";
    case "tif":
    case "tiff":
      return "image/tiff";
    case "bmp":
      return "image/bmp";
    default:
      return null;
  }
}

export function localContextToAttachment(context: PickerTextContext): ChatAttachment {
  return {
    name: context.name,
    mime_type: context.mime_type,
    byte_count: context.byte_count,
    text: [
      "Source: native picker grant (consumed)",
      `Truncated: ${context.truncated ? "yes" : "no"}`,
      "",
      context.text,
    ].join("\n"),
  };
}

export function isSupportedVisualChatAttachment(
  attachment: Pick<ChatAttachment, "name" | "mime_type">,
) {
  return (
    supportedVisualAttachmentMimePattern.test(attachment.mime_type) ||
    supportedVisualAttachmentExtensionPattern.test(attachment.name)
  );
}

export function shouldAnalyzeVisualChatAttachment(attachment: ChatAttachment) {
  return (
    isSupportedVisualChatAttachment(attachment) &&
    Boolean(attachment.data_base64?.trim()) &&
    !attachment.text?.trim()
  );
}

export function visualAnalysisRequestForAttachment(attachment: ChatAttachment) {
  return {
    dataBase64: attachment.data_base64,
    fileName: attachment.name,
    mimeType: supportedVisualAttachmentMimePattern.test(attachment.mime_type)
      ? attachment.mime_type
      : mimeTypeForVisualFileName(attachment.name) ?? attachment.mime_type,
  };
}

export function visualAnalysisTextForAttachment(
  attachment: ChatAttachment,
  analysis: VisualArtifactAnalysis,
) {
  const promptContext = analysis.prompt_context?.trim();
  if (promptContext) {
    return promptContext;
  }
  return [
    `Visual analysis for ${analysis.artifact_name || attachment.name}`,
    `MIME type: ${analysis.mime_type || attachment.mime_type}`,
    `Backend: ${analysis.backend || "local visual analyzer"}`,
    "",
    "Detected text:",
    ...(analysis.extracted_text?.length
      ? analysis.extracted_text.map((text) => `- ${text}`)
      : ["- No readable text was detected."]),
    "",
    "Detected visual content:",
    ...(analysis.classifications?.length
      ? analysis.classifications.map((label) => `- ${label}`)
      : ["- No visual classification labels were returned."]),
  ].join("\n");
}

function attachmentReceipt(attachments: ChatAttachment[]) {
  if (attachments.length === 0) {
    return "";
  }
  return [
    "Attached files:",
    ...attachments.map(
      (attachment) =>
        `- ${attachment.name} (${attachment.mime_type}; ${attachment.byte_count} bytes)`,
    ),
  ].join("\n");
}

export function messageWithAttachmentReceipt(
  content: string,
  attachments: ChatAttachment[],
) {
  const receipt = attachmentReceipt(attachments);
  return [content.trim(), receipt].filter(Boolean).join("\n\n");
}

export function releaseAttachmentPayloads(attachments: ChatAttachment[]) {
  for (const attachment of attachments) {
    attachment.data_base64 = undefined;
    attachment.text = undefined;
    attachment.approved_file_receipt = undefined;
    attachment.private_data_provenance = undefined;
  }
  attachments.length = 0;
}
