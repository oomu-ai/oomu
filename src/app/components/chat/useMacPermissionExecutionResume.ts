"use client";

import { invoke } from "@/lib/invoke";
import { useCallback, useEffect, useRef } from "react";
import type { ActiveAgentExecution } from "./agentExecutionState";
import { resumablePermissionCapability } from "./RecoveryReceiptCard";
import { consumeMacPermissionRefresh } from "./macPermissionRefreshSignal";

type PermissionResumeResponse = {
  resumed: boolean;
  executionId?: string;
  reason: "resumed" | "permission_not_allowed" | "no_waiting_work" | "choose_waiting_work" | "already_resumed";
};

type MacPermissionStatus = {
  capabilityId: string;
  state: "not_requested" | "allowed" | "limited" | "denied" | "restricted" | "requires_settings" | "stale" | "unsupported";
};

const OPERATION_VERIFIED_CAPABILITIES = new Set([
  "full_disk_access",
  "local_network",
  "mail",
  "notes",
  "messages",
  "finder",
  "system_events",
]);

function permissionResumeCapability(
  messages: ReadonlyArray<{ content: string }>,
  executionId: string,
) {
  if (!executionId) return "";
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const capabilityId = resumablePermissionCapability(messages[index].content, executionId);
    if (capabilityId) return capabilityId;
  }
  return "";
}

export function useMacPermissionExecutionResume({
  activeExecution,
  activeSessionId,
  messages,
  onResumed,
}: {
  activeExecution: ActiveAgentExecution | null;
  activeSessionId: string;
  messages: ReadonlyArray<{ content: string }>;
  onResumed: (executionId: string) => void;
}) {
  const onResumedRef = useRef(onResumed);
  const inFlightRef = useRef(false);
  useEffect(() => { onResumedRef.current = onResumed; }, [onResumed]);
  const executionId = activeExecution?.sessionId === activeSessionId
    && activeExecution.status === "halted" ? activeExecution.executionId : "";
  const candidateCapabilityId = permissionResumeCapability(messages, executionId);

  const attemptResume = useCallback(async () => {
    if (!candidateCapabilityId || !executionId || inFlightRef.current) return;
    inFlightRef.current = true;
    try {
      const statuses = await invoke<MacPermissionStatus[]>("list_macos_permission_states");
      const current = statuses.find(({ capabilityId }) => capabilityId === candidateCapabilityId);
      if (!OPERATION_VERIFIED_CAPABILITIES.has(candidateCapabilityId) && current?.state !== "allowed") {
        return;
      }
      const response = await invoke<PermissionResumeResponse>(
        "resume_agent_execution_after_permission",
        { request: { capabilityId: candidateCapabilityId, executionId } },
      );
      if (response.resumed) onResumedRef.current(executionId);
    } catch {
      // The permission panel owns calm user-facing recovery copy. A denied or
      // unavailable permission remains halted and will be checked again after
      // the user returns from System Settings.
    } finally {
      inFlightRef.current = false;
    }
  }, [candidateCapabilityId, executionId]);

  useEffect(() => {
    if (!candidateCapabilityId || !executionId) return;
    const handlePermissionRefresh = (event: Event) => {
      const refreshedCapability = (event as CustomEvent<{ capabilityId?: string }>).detail?.capabilityId;
      if (refreshedCapability !== candidateCapabilityId) return;
      consumeMacPermissionRefresh(candidateCapabilityId);
      void attemptResume();
    };
    const handleFocus = () => void attemptResume();
    if (
      consumeMacPermissionRefresh(candidateCapabilityId)
      || !OPERATION_VERIFIED_CAPABILITIES.has(candidateCapabilityId)
    ) {
      void attemptResume();
    }
    window.addEventListener("oomu:macos-permissions-refreshed", handlePermissionRefresh);
    window.addEventListener("focus", handleFocus);
    return () => {
      window.removeEventListener("oomu:macos-permissions-refreshed", handlePermissionRefresh);
      window.removeEventListener("focus", handleFocus);
    };
  }, [attemptResume, candidateCapabilityId, executionId]);
}
