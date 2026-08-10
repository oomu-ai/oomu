import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatTurnContext } from "@/lib/chatTurnContext";
import { ATTACHMENT_LIMITS } from "@/lib/attachmentProcessing";
import { approvedLocalFileAttachments } from "./directLocalFileRead";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: (command: string, args?: unknown) => invokeMock(command, args),
}));

const turn: ChatTurnContext = {
  turnId: "turn-1",
  generationToken: "generation-1",
  sessionId: "session-1",
  agentId: "agent-1",
  projectId: null,
  ancestry: {
    kind: "root",
    parentTurnId: null,
    rootTurnId: "turn-1",
  },
  route: {
    providerId: "provider-1",
    modelId: "model-1",
    reasoning: "medium",
    primaryRouteId: null,
    fallbackRouteId: null,
    dynamicRoutingEnabled: false,
    automatedWebGroundingEnabled: false,
  },
  attachmentGrants: [],
  createdAtMs: 1,
};

function preparedFile(path: string, byteCount = 24) {
  const name = path.split("/").at(-1) ?? "file.txt";
  return {
    displayName: name,
    mimeType: "text/plain",
    byteCount,
    receipt: {
      payload: `receipt-${name}`,
      signature: {
        public_key: "public-key",
        signature: `signature-${name}`,
        payload_hash: "payload-hash",
        signed_at_ms: 1,
      },
    },
  };
}

describe("approved local file batches", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (_command: string, args?: {
      request?: { access?: { action?: { path?: string } } };
    }) => preparedFile(args?.request?.access?.action?.path ?? "file.txt"));
  });

  it("preflights the whole count before preparing any file", async () => {
    const paths = Array.from({ length: ATTACHMENT_LIMITS.maxCount + 1 }, (_, index) =>
      `/Users/example/Documents/file-${index}.txt`
    );

    await expect(approvedLocalFileAttachments(paths, "Compare these files", turn, []))
      .rejects.toMatchObject({ code: "approved_file_attachment_limit" });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("prepares a duplicate path only once", async () => {
    const path = "/Users/example/Documents/report.txt";

    const attachments = await approvedLocalFileAttachments(
      [path, path],
      "Review the report",
      turn,
      [],
    );

    expect(attachments).toHaveLength(1);
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it("stops an over-limit batch before model dispatch can receive it", async () => {
    invokeMock
      .mockResolvedValueOnce(preparedFile("/Users/example/Documents/one.txt", 8 * 1024 * 1024))
      .mockResolvedValueOnce(preparedFile("/Users/example/Documents/two.txt", 8 * 1024 * 1024));
    const existing = [{
      name: "existing.bin",
      mime_type: "application/octet-stream",
      byte_count: 5 * 1024 * 1024,
      data_base64: "AA==",
    }];

    await expect(approvedLocalFileAttachments([
      "/Users/example/Documents/one.txt",
      "/Users/example/Documents/two.txt",
    ], "Compare these files", turn, existing)).rejects.toMatchObject({
      code: "approved_file_unavailable",
    });
  });
});
