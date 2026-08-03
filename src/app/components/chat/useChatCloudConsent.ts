import { usePrivateEgressConsent } from "./usePrivateEgressConsent";
import { useProjectCloudConsent } from "./useProjectCloudConsent";
import { useStableEvent } from "./sessionScopedState";

type UseChatCloudConsentOptions = {
  activeSessionId: string;
  t: (key: string) => string;
  setSendingForSession: (sessionId: string, value: boolean) => void;
  setProcessingForSession: (sessionId: string, value: boolean) => void;
  setStatusForSession: (sessionId: string, value: string) => void;
};

export function useChatCloudConsent(options: UseChatCloudConsentOptions) {
  const shared = {
    activeSessionId: options.activeSessionId,
    setSendingForSession: options.setSendingForSession,
    setProcessingForSession: options.setProcessingForSession,
    setStatusForSession: options.setStatusForSession,
  };
  const project = useProjectCloudConsent({
    ...shared,
    attentionStatus: options.t("chat.project_cloud_consent.waiting_status"),
    resumingStatus: options.t("chat.project_cloud_consent.resuming_status"),
  });
  const privateEgress = usePrivateEgressConsent({
    ...shared,
    attentionStatus: options.t("chat.private_egress_consent.waiting_status"),
    resumingStatus: options.t("chat.private_egress_consent.resuming_status"),
    keptPrivateStatus: options.t("chat.private_egress_consent.kept_private_status"),
  });
  const cancelChatCloudConsentForSession = useStableEvent((sessionId: string) => {
    project.cancelProjectCloudConsentForSession(sessionId);
    privateEgress.cancelPrivateEgressConsentForSession(sessionId);
  });
  return { ...project, ...privateEgress, cancelChatCloudConsentForSession };
}
