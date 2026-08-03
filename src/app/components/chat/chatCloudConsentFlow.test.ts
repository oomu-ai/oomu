import { beforeEach, describe, expect, it, vi } from "vitest";
import { createChatTurnContext } from "@/lib/chatTurnContext";
import { resolveChatCloudConsentBoundary } from "./chatCloudConsentFlow";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }));

const turn = createChatTurnContext({
  turnId: "turn-1",
  generationToken: "generation-1",
  sessionId: "session-1",
  agentId: "agent-1",
  route: {
    providerId: "gemini",
    modelId: "gemini-3.6-flash",
    dynamicRoutingEnabled: false,
    automatedWebGroundingEnabled: false,
  },
});

const baseOptions = {
  error: { code: "private_egress_confirmation_required" },
  turn,
  projectId: null,
  projectDestination: "Gemini 3.6 Flash",
  privateDestination: (_providerId: string, modelId: string) => modelId,
  requestProjectConsent: vi.fn(),
};

describe("chat cloud consent flow", () => {
  beforeEach(() => invokeMock.mockReset());

  it("resumes the same turn after one explicit private-source approval", async () => {
    invokeMock.mockResolvedValueOnce({
      challengeId: "challenge-1",
      destinationProviderId: "gemini",
      destinationModelId: "gemini-3.6-flash",
      sourceNames: ["plan.md"],
    }).mockResolvedValueOnce({ decision: "approved" });
    const requestPrivateConsent = vi.fn().mockResolvedValue("send_once");

    await expect(resolveChatCloudConsentBoundary({
      ...baseOptions,
      requestPrivateConsent,
    })).resolves.toBe(false);
    expect(requestPrivateConsent).toHaveBeenCalledWith(turn, {
      challengeId: "challenge-1",
      destination: "gemini-3.6-flash",
      sourceNames: ["plan.md"],
    });
    expect(invokeMock).toHaveBeenLastCalledWith("resolve_private_egress_confirmation", {
      request: {
        challengeId: "challenge-1",
        sessionId: "session-1",
        turnId: "turn-1",
        generationToken: "generation-1",
        approved: true,
      },
    });
  });

  it("keeps the source private and stops cleanly when the user declines", async () => {
    invokeMock.mockResolvedValueOnce({
      challengeId: "challenge-2",
      destinationProviderId: "gemini",
      destinationModelId: "gemini-3.6-flash",
      sourceNames: ["private.md"],
    }).mockResolvedValueOnce({ decision: "denied" });

    await expect(resolveChatCloudConsentBoundary({
      ...baseOptions,
      requestPrivateConsent: vi.fn().mockResolvedValue("keep_private"),
    })).rejects.toMatchObject({ code: "private_egress_user_denied" });
    expect(invokeMock).toHaveBeenLastCalledWith(
      "resolve_private_egress_confirmation",
      expect.objectContaining({ request: expect.objectContaining({ approved: false }) }),
    );
  });

  it("leaves unrelated failures for the normal chat error path", async () => {
    await expect(resolveChatCloudConsentBoundary({
      ...baseOptions,
      error: { code: "provider_network_error" },
      requestPrivateConsent: vi.fn(),
    })).resolves.toBeNull();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("never manufactures a consent challenge for a missing native egress receipt", async () => {
    const requestPrivateConsent = vi.fn();

    await expect(resolveChatCloudConsentBoundary({
      ...baseOptions,
      error: { code: "private_egress_receipt_required" },
      requestPrivateConsent,
    })).resolves.toBeNull();

    expect(requestPrivateConsent).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
