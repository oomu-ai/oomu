export type AttachmentFailure = {
  name: string;
  errorCode: string | null;
};

type Translate = (key: string, values?: Record<string, string | number>) => string;

export function composerAttachmentIsSupported(mimeType: string) {
  const normalized = mimeType.trim().toLowerCase();
  return normalized.startsWith("text/")
    || normalized.startsWith("image/")
    || [
      "application/json",
      "application/pdf",
      "application/typescript",
      "application/xml",
      "application/x-yaml",
    ].includes(normalized);
}

export function attachmentFailureCopy(failure: AttachmentFailure, t: Translate) {
  const code = failure.errorCode ?? "";
  if (code === "attachment_format_unsupported") {
    return t("chat.attachment_errors.unsupported", { name: failure.name });
  }
  if (code.includes("byte_limit") || code.includes("dimension_limit")) {
    return t("chat.attachment_errors.too_large", { name: failure.name });
  }
  if (code === "attachment_count_limit_exceeded") {
    return t("chat.attachment_errors.too_many");
  }
  return t("chat.attachment_errors.unavailable", { name: failure.name });
}
