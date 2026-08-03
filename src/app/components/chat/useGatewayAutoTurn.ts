import { useEffect, useRef } from "react";
import { invoke, isTauriRuntime } from "@/lib/invoke";
import type { ChatSession } from "@/lib/chatSessions";

type AutoTurnGatewayEvent = {
  sessionId: string;
  status:
    | "retrieving"
    | "processing"
    | "completed"
    | "failed"
    | "data_retrying";
};

type GatewayAutoTurnOptions = {
  translate: (key: string) => string;
  setExecuting: (sessionId: string, active: boolean) => void;
  setProcessing: (sessionId: string, active: boolean) => void;
  setSending: (sessionId: string, active: boolean) => void;
  setStatus: (sessionId: string, status: string) => void;
  refreshSessionMessages: (sessionId: string) => Promise<unknown>;
  onSessionsChange: (sessions: ChatSession[]) => void;
};

export function useGatewayAutoTurn(options: GatewayAutoTurnOptions) {
  const callbacksRef = useRef(options);

  useEffect(() => {
    callbacksRef.current = options;
  }, [options]);

  useEffect(() => {
    if (!isTauriRuntime) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<AutoTurnGatewayEvent>("gateway://auto-turn", (event) => {
          if (cancelled) return;
          const payload = event.payload;
          const sessionId = payload.sessionId?.trim();
          if (!sessionId) return;
          const callbacks = callbacksRef.current;

          if (payload.status === "data_retrying") {
            callbacks.setProcessing(sessionId, true);
            callbacks.setStatus(
              sessionId,
              callbacks.translate("gateway.security.data_error"),
            );
            return;
          }
          if (payload.status === "retrieving" || payload.status === "processing") {
            callbacks.setExecuting(sessionId, true);
            callbacks.setProcessing(sessionId, true);
            callbacks.setStatus(
              sessionId,
              callbacks.translate(
                payload.status === "retrieving"
                  ? "gateway.auto_turn.retrieving"
                  : "gateway.auto_turn.processing",
              ),
            );
            return;
          }

          callbacks.setExecuting(sessionId, false);
          callbacks.setProcessing(sessionId, false);
          callbacks.setSending(sessionId, false);
          callbacks.setStatus(
            sessionId,
            callbacks.translate(
              payload.status === "completed"
                ? "gateway.auto_turn.complete"
                : "gateway.auto_turn.failed",
            ),
          );
          if (payload.status === "completed") {
            void callbacks.refreshSessionMessages(sessionId).catch(() => undefined);
            void invoke<ChatSession[]>("list_chat_sessions")
              .then((sessions) => callbacks.onSessionsChange(sessions))
              .catch(() => undefined);
          }
        }),
      )
      .then((cleanup) => {
        if (cancelled) cleanup();
        else unlisten = cleanup;
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
}
