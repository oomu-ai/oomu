import {
  AutoRouteAttentionCard,
  type AutoRouteAttention,
  type AutoRouteRecoveryAction,
} from "./AutoRouteAttentionCard";
import {
  MacPermissionRecoveryCard,
  type MacPermissionRecoveryDescriptor,
} from "./MacPermissionRecoveryCard";

type Translate = (key: string, variables?: Record<string, string | number>) => string;

type PermissionAttention = {
  boundary: string;
  code: string;
  descriptor: MacPermissionRecoveryDescriptor;
  sessionId: string;
  turnId: string;
};

type PermissionActions = {
  onCancel: (recoveryId: string) => Promise<void>;
  onCheck: (recoveryId: string, capabilityId: string) => Promise<void>;
  onOpenSettings: (recoveryId: string, capabilityId: string) => Promise<void>;
};

export function ChatTurnRecoveryCards({
  activeSessionId,
  autoRouteAttention,
  directApplePermissionActions,
  directApplePermissionAttention,
  onAutoRouteChoice,
  t,
}: {
  activeSessionId: string;
  autoRouteAttention: AutoRouteAttention | null;
  directApplePermissionActions: PermissionActions;
  directApplePermissionAttention: PermissionAttention | null;
  onAutoRouteChoice: (choice: AutoRouteRecoveryAction) => void | Promise<void>;
  t: Translate;
}) {
  return (
    <>
      {autoRouteAttention?.sessionId === activeSessionId ? (
        <AutoRouteAttentionCard attention={autoRouteAttention} onChoice={onAutoRouteChoice} t={t} />
      ) : null}
      {directApplePermissionAttention?.sessionId === activeSessionId ? (
        <MacPermissionRecoveryCard
          boundary={directApplePermissionAttention.boundary}
          code={directApplePermissionAttention.code}
          descriptor={directApplePermissionAttention.descriptor}
          onCancel={directApplePermissionActions.onCancel}
          onCheck={directApplePermissionActions.onCheck}
          onOpenSettings={directApplePermissionActions.onOpenSettings}
          recoveryId={directApplePermissionAttention.turnId}
          t={t}
        />
      ) : null}
    </>
  );
}
