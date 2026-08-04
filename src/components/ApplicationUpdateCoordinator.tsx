"use client";

import type { MutableRefObject } from "react";
import { useApplicationUpdates } from "@/hooks/useApplicationUpdates";
import { ApplicationUpdateDialog } from "./ApplicationUpdateDialog";

type ApplicationUpdateCoordinatorProps = {
  navigationGuard: MutableRefObject<((proceed: () => void) => void) | null>;
  presentationBlocked: boolean;
};

export function ApplicationUpdateCoordinator({
  navigationGuard,
  presentationBlocked,
}: ApplicationUpdateCoordinatorProps) {
  const updates = useApplicationUpdates(navigationGuard);
  return (
    <ApplicationUpdateDialog
      onCheck={() => void updates.retry()}
      onDismiss={updates.dismiss}
      onInstall={() => void updates.install()}
      onOpenFullNotes={() => void updates.openFullNotes()}
      onRemind={() => void updates.remind()}
      onRestart={updates.restart}
      onSkip={() => void updates.skip()}
      presentationBlocked={presentationBlocked || !updates.uiReady}
      view={updates.view}
    />
  );
}
