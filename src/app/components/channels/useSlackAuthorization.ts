"use client";

import { useEffect } from "react";
import { integrationApi } from "../integrations/integrationClient";

export type SlackAuthorizationAttempt = {
  connectorId: string;
  expiresAtMs: number;
  startedAtMs: number;
};

export type SlackAuthorizationOutcome =
  | "pending"
  | "complete"
  | "denied"
  | "expired"
  | "workspace_restricted"
  | "failed";

const POLL_INTERVAL_MS = 2_000;

function failureOutcome(
  code?: string,
): Exclude<SlackAuthorizationOutcome, "pending"> {
  if (code === "slack_authorization_access_denied") return "denied";
  if (code === "slack_authorization_expired") return "expired";
  if (code === "slack_authorization_workspace_restricted") return "workspace_restricted";
  return "failed";
}

export function useSlackAuthorization(
  attempt: SlackAuthorizationAttempt | null,
  onOutcome: (outcome: Exclude<SlackAuthorizationOutcome, "pending">) => void,
) {
  useEffect(() => {
    if (!attempt) return;
    let cancelled = false;
    let timer: number | undefined;

    const poll = async () => {
      if (Date.now() >= attempt.expiresAtMs) {
        if (!cancelled) onOutcome("expired");
        return;
      }
      try {
        const status = await integrationApi.connectionStatus(attempt.connectorId);
        if (cancelled) return;
        if (status.grantedScopes.includes("chat:write")
          && ["authorized", "reachable"].includes(status.connectionState)) {
          onOutcome("complete");
          return;
        }
        const failureIsCurrent = Boolean(
          status.lastProbeAtMs
          && status.lastProbeAtMs >= attempt.startedAtMs - 1_000
          && status.lastProbeCode
          && !["oauth_started", "oauth_completed"].includes(status.lastProbeCode),
        );
        if (status.connectionState === "disconnected" || failureIsCurrent) {
          onOutcome(failureOutcome(status.lastProbeCode));
          return;
        }
      } catch {
        // A transient read failure must not interrupt an in-progress Slack install.
      }
      timer = window.setTimeout(() => void poll(), POLL_INTERVAL_MS);
    };

    timer = window.setTimeout(() => void poll(), 0);
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [attempt, onOutcome]);
}
