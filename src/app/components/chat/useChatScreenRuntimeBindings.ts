import { useCallback, useEffect, useMemo } from "react";
import type { ChatSession } from "@/lib/chatSessions";
import type { ChatTranslate } from "./chatFailureNotice";
import type { ChatSessionRouteBinding } from "./sessionRouting";

type CreateChatSession = (
  agentId: string,
  route: ChatSessionRouteBinding,
  projectId?: string | null,
) => Promise<ChatSession | null>;

type CancelRemoteMcpOperations = (serverName?: string) => Promise<number>;

export type VerifiedExecutionCopy = {
  failurePrefix: string;
  receiptPrefix: string;
  toolFailureWithoutDetails: string;
  toolResultMissing: string;
  fileChangedBeforeSave: string;
  filePreparationFailed: string;
  fileVerificationFailed: string;
};

export function useProjectScopedChatSessionCreator(
  createSession: CreateChatSession,
  projectId: string | null,
) {
  return useCallback(
    (agentId: string, route: ChatSessionRouteBinding) =>
      projectId
        ? createSession(agentId, route, projectId)
        : createSession(agentId, route),
    [createSession, projectId],
  );
}

export function useVerifiedExecutionCopy(translate: ChatTranslate) {
  return useMemo<VerifiedExecutionCopy>(() => ({
    failurePrefix: translate("trust.local_command_failed"),
    receiptPrefix: translate("trust.verified_native_receipt"),
    toolFailureWithoutDetails: translate("trust.local_tool_failure_no_details"),
    toolResultMissing: translate("trust.local_tool_no_verifiable_result"),
    fileChangedBeforeSave: translate("trust.file_changed_before_save"),
    filePreparationFailed: translate("trust.file_preparation_failed"),
    fileVerificationFailed: translate("trust.file_verification_failed"),
  }), [translate]);
}

export function useRemoteMcpCancellation(
  sessionId: string,
  cancelRemoteOperations?: CancelRemoteMcpOperations,
) {
  useEffect(
    () => () => {
      void cancelRemoteOperations?.();
    },
    [cancelRemoteOperations, sessionId],
  );
}
