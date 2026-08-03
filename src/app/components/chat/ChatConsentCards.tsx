import type { PrivateEgressConsentAttention } from "./PrivateEgressConsentCard";
import {
  PrivateEgressConsentCard,
  type PrivateEgressConsentChoice,
} from "./PrivateEgressConsentCard";
import type {
  ProjectCloudConsentAttention,
  ProjectCloudConsentChoice,
} from "./ProjectCloudConsentCard";
import { ProjectCloudConsentCard } from "./ProjectCloudConsentCard";

type ChatConsentCardsProps = {
  activeSessionId: string;
  consent: {
    privateEgressConsentAttention: PrivateEgressConsentAttention | null;
    projectCloudConsentAttention: ProjectCloudConsentAttention | null;
    resolvePrivateEgressConsent: (choice: PrivateEgressConsentChoice) => void;
    resolveProjectCloudConsent: (choice: ProjectCloudConsentChoice) => void;
  };
  t: (key: string, variables?: Record<string, string | number>) => string;
};

export function ChatConsentCards({
  activeSessionId,
  consent,
  t,
}: ChatConsentCardsProps) {
  return (
    <>
      {consent.projectCloudConsentAttention?.sessionId === activeSessionId && (
        <ProjectCloudConsentCard
          attention={consent.projectCloudConsentAttention}
          onChoice={consent.resolveProjectCloudConsent}
          t={t}
        />
      )}
      {consent.privateEgressConsentAttention?.sessionId === activeSessionId && (
        <PrivateEgressConsentCard
          attention={consent.privateEgressConsentAttention}
          onChoice={consent.resolvePrivateEgressConsent}
          t={t}
        />
      )}
    </>
  );
}
