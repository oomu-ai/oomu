import { useEffect, useRef, useState } from "react";
import { useI18n } from "@/context/I18nContext";
import { localizedOAuthFailure, oauthConnectionOutcome } from "./connectorOAuthStatus";
import { integrationApi, type ConnectorManifest } from "./integrationClient";

export type ConnectorOAuthTarget = {
  connectorId?: string;
  operation?: string;
  scopes: string[];
};

type ReconnectActionState = {
  connectorId: string;
  kind: "reconnect";
  state: "working" | "success" | "error";
  messageKey: string;
};

export function useConnectorOAuthFlow({
  manifest,
  onAct,
  onRefresh,
  onReconnectState,
  onStarted,
}: {
  manifest: ConnectorManifest;
  onAct: (action: () => Promise<unknown>, errorCode?: string) => Promise<boolean>;
  onRefresh: () => Promise<void>;
  onReconnectState: (state: ReconnectActionState | null) => void;
  onStarted: () => void;
}) {
  const { t } = useI18n();
  const inFlightRef = useRef(false);
  const [pending, setPending] = useState<{
    connectorId: string;
    reconnectConnectorId?: string;
  } | null>(null);
  const [statusError, setStatusError] = useState("");

  async function start(target: ConnectorOAuthTarget) {
    if (inFlightRef.current) return;
    inFlightRef.current = true;
    const connectorId = target.connectorId;
    onReconnectState(connectorId
      ? { connectorId, kind: "reconnect", state: "working", messageKey: "reconnecting" }
      : null);
    setStatusError("");
    try {
      let attempt: Awaited<ReturnType<typeof integrationApi.connect>> | undefined;
      const succeeded = await onAct(async () => {
        attempt = await integrationApi.connect(
          manifest.manifestId,
          connectorId,
          target.operation ? [target.operation] : undefined,
        );
      }, "connect_failed");
      if (succeeded && attempt) {
        onStarted();
        setPending({
          connectorId: attempt.connectorId,
          reconnectConnectorId: connectorId,
        });
      } else if (connectorId) {
        onReconnectState({
          connectorId,
          kind: "reconnect",
          state: "error",
          messageKey: "reconnect_failed",
        });
      }
    } finally {
      inFlightRef.current = false;
    }
  }

  useEffect(() => {
    if (!pending) return;
    let cancelled = false;
    let polling = false;
    const poll = async () => {
      if (polling) return;
      polling = true;
      try {
        const status = await integrationApi.connectionStatus(pending.connectorId);
        if (cancelled) return;
        const outcome = oauthConnectionOutcome(status);
        if (outcome === "pending") return;
        setPending(null);
        if (outcome === "failed") {
          onReconnectState(null);
          setStatusError(localizedOAuthFailure(status, t));
        } else {
          setStatusError("");
          if (pending.reconnectConnectorId) {
            onReconnectState({
              connectorId: pending.reconnectConnectorId,
              kind: "reconnect",
              state: "success",
              messageKey: "connection_checked",
            });
          }
        }
        await onRefresh();
      } catch {
        if (!cancelled) {
          setStatusError(t("setup.errors.setup_connector_status_failed"));
        }
      } finally {
        polling = false;
      }
    };
    const timer = window.setInterval(() => void poll(), 1_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [onReconnectState, onRefresh, pending, t]);

  return { pending: Boolean(pending), start, statusError };
}
