import { describe, expect, it } from "vitest";
import {
  attachmentFailureCopy,
  composerAttachmentIsSupported,
} from "./attachmentFailureCopy";

const t = (key: string, values?: Record<string, string | number>) =>
  `${key}:${values?.name ?? ""}`;

describe("attachment failure copy", () => {
  it("accepts the real composer formats and rejects unknown binary files", () => {
    expect(composerAttachmentIsSupported("text/markdown")).toBe(true);
    expect(composerAttachmentIsSupported("application/json")).toBe(true);
    expect(composerAttachmentIsSupported("image/png")).toBe(true);
    expect(composerAttachmentIsSupported("application/octet-stream")).toBe(false);
  });

  it("keeps the failed filename in actionable user copy", () => {
    expect(attachmentFailureCopy({
      name: "unsupported.oomu-invalid",
      errorCode: "attachment_format_unsupported",
    }, t)).toBe("chat.attachment_errors.unsupported:unsupported.oomu-invalid");
  });
});
