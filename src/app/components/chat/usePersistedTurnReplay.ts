import { useStableEvent } from "./sessionScopedState";
import type { AutoRouteAttention, AutoRouteTurnChoice } from "./AutoRouteAttentionCard";
import type { DirectApplePermissionAttention } from "./useDirectApplePermissionRecovery";

export type PersistedTurnReplayMessage = {
  content: string;
  role: string;
  providerId?: string | null;
  modelId?: string | null;
  metadata?: {
    turnId?: string;
    rootTurnId?: string;
    generationToken?: string;
    turnState?: string;
    terminalResultForTurnId?: string;
    permissionContinuation?: {
      state: "waiting" | "retrying" | "completed";
      capabilityId: string;
      errorCode?: string;
      boundary?: string;
    };
  } | null;
};

export type PersistedAcceptedTurnIdentity = {
  sessionId: string;
  rootTurnId: string;
  turnId: string;
  generationToken: string;
  providerId: string;
  modelId: string;
  turnState: "accepted" | "interrupted";
  dynamicRoutingEnabled: boolean;
};

export type PersistedTurnReplaySubmitOptions = {
  autoRouteResumeChoice?: "local" | "cloud" | null;
  onAccepted?: () => void;
  resumeAcceptedTurn?: PersistedAcceptedTurnIdentity;
};

type Submit = (message: string, options: PersistedTurnReplaySubmitOptions) => Promise<void>;

export function usePersistedTurnReplay({
  activeSessionId,
  messages,
  submit,
}: {
  activeSessionId: string;
  messages: readonly PersistedTurnReplayMessage[];
  submit: Submit;
}) {
  const replay = useStableEvent(async (
    identity: { sessionId: string; rootTurnId: string; turnId: string; generationToken: string },
    autoRouteResumeChoice?: "local" | "cloud" | null,
    dynamicRoutingEnabled: boolean = false,
  ) => {
    if (identity.sessionId !== activeSessionId.trim()) return false;
    const acceptedMessage = messages.find((candidate) =>
      candidate.role === "user"
      && candidate.metadata?.turnId === identity.turnId
      && (candidate.metadata?.rootTurnId ?? candidate.metadata?.turnId) === identity.rootTurnId
      && candidate.metadata?.generationToken === identity.generationToken
      && ["accepted", "interrupted"].includes(candidate.metadata.turnState ?? "")
    );
    const message = acceptedMessage?.content ?? "";
    const providerId = acceptedMessage?.providerId?.trim() ?? "";
    const modelId = acceptedMessage?.modelId?.trim() ?? "";
    const turnState = acceptedMessage?.metadata?.turnState;
    if (!message || !providerId || !modelId
      || (turnState !== "accepted" && turnState !== "interrupted")) return false;
    let accepted = false;
    await submit(message, {
      autoRouteResumeChoice,
      onAccepted: () => { accepted = true; },
      resumeAcceptedTurn: {
        sessionId: identity.sessionId,
        rootTurnId: identity.rootTurnId,
        turnId: identity.turnId,
        generationToken: identity.generationToken,
        providerId,
        modelId,
        turnState,
        dynamicRoutingEnabled,
      },
    });
    return accepted;
  });

  const resumeAutoRouteTurn = useStableEvent((
    attention: AutoRouteAttention,
    choice: Exclude<AutoRouteTurnChoice, "cancel">,
  ) => replay(attention, choice === "retry" ? null : choice, true));
  const resumeApplePermissionTurn = useStableEvent(
    (attention: DirectApplePermissionAttention) => replay(attention),
  );
  return { resumeApplePermissionTurn, resumeAutoRouteTurn };
}
