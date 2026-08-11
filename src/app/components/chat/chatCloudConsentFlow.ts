import { invoke } from "@/lib/invoke";
import type { ChatTurnContext } from "@/lib/chatTurnContext";
import type { PrivateEgressConsentChoice } from "./PrivateEgressConsentCard";
import type { ProjectCloudConsentChoice } from "./ProjectCloudConsentCard";
import { stableErrorCode } from "./inferenceErrors";

type PrivateEgressConfirmation = {
  challengeId: string;
  destinationProviderId: string;
  destinationModelId: string;
  sourceNames: string[];
};

type ChatCloudConsentFlowOptions = {
  error: unknown;
  turn: ChatTurnContext;
  projectId?: string | null;
  projectDestination: string;
  privateDestination: (providerId: string, modelId: string) => string;
  requestProjectConsent: (
    turn: ChatTurnContext,
    destination: string,
  ) => Promise<ProjectCloudConsentChoice>;
  requestPrivateConsent: (
    turn: ChatTurnContext,
    attention: {
      challengeId: string;
      destination: string;
      sourceNames: string[];
    },
  ) => Promise<PrivateEgressConsentChoice>;
};

export type ChatCloudConsentResolution =
  | { kind: "private_egress_approved" }
  | { kind: "project_provider_approved" }
  | { kind: "project_provider_confirmation_invalid" };

export function chatCloudConsentContinuation(
  resolution: ChatCloudConsentResolution,
  dynamicRoutingEnabled: boolean,
  currentChoice: "local" | "cloud" | null,
): ["local" | "cloud" | null, boolean] {
  return [
    resolution.kind === "private_egress_approved" && dynamicRoutingEnabled
      ? "cloud"
      : currentChoice,
    resolution.kind === "project_provider_approved",
  ];
}

export async function resolveChatCloudConsentBoundary({
  error,
  turn,
  projectId,
  projectDestination,
  privateDestination,
  requestProjectConsent,
  requestPrivateConsent,
}: ChatCloudConsentFlowOptions): Promise<ChatCloudConsentResolution | null> {
  const code = stableErrorCode(error);
  if (code === "private_egress_confirmation_required") {
    const confirmation = await invoke<PrivateEgressConfirmation>(
      "get_private_egress_confirmation",
      {
        request: {
          sessionId: turn.sessionId,
          turnId: turn.turnId,
          generationToken: turn.generationToken,
        },
      },
    );
    const choice = await requestPrivateConsent(turn, {
      challengeId: confirmation.challengeId,
      destination: privateDestination(
        confirmation.destinationProviderId,
        confirmation.destinationModelId,
      ),
      sourceNames: confirmation.sourceNames,
    });
    await invoke("resolve_private_egress_confirmation", {
      request: {
        challengeId: confirmation.challengeId,
        sessionId: turn.sessionId,
        turnId: turn.turnId,
        generationToken: turn.generationToken,
        approved: choice === "send_once",
      },
    });
    if (choice === "keep_private") {
      throw { code: "private_egress_user_denied" };
    }
    return { kind: "private_egress_approved" };
  }
  if (code === "project_provider_confirmation_invalid") {
    return { kind: "project_provider_confirmation_invalid" };
  }
  if (code !== "project_provider_consent_required" || !projectId) {
    return null;
  }
  const choice = await requestProjectConsent(turn, projectDestination);
  if (choice === "cancel") {
    throw { code: "project_cloud_choice_cancelled" };
  }
  if (choice === "always") {
    await invoke("set_project_policy", {
      request: { projectId, dataPolicy: "allow_configured_cloud" },
    });
  }
  return { kind: "project_provider_approved" };
}
