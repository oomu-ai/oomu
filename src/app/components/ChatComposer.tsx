"use client";

import { useI18n } from "@/context/I18nContext";
import { invoke } from "@/lib/invoke";
import { listen } from "@tauri-apps/api/event";
import type { ChatSubmissionOutcome } from "./chat/submissionAcceptance";
import { useNativeComposerDrop } from "./chat/useNativeComposerDrop";
import {
  memo,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";

type ChatComposerAttachment = {
  name: string;
  byte_count: number;
};

type VoiceCaptureStatus = {
  capture_id?: string;
  captureId?: string;
  active: boolean;
};

type VoiceStreamEvent = {
  capture_id?: string;
  captureId?: string;
  text?: string;
  is_final?: boolean;
  isFinal?: boolean;
  error_code?: string;
  errorCode?: string;
};

function releaseEventListener(unlisten: (() => void) | null) {
  if (!unlisten) return;
  try {
    // Tauri types this as synchronous, but its implementation is async. A
    // webview reload can remove the native listener before React's cleanup
    // runs, making that promise reject even though the desired end state has
    // already been reached. Treat cleanup as idempotent and contain the stale
    // listener rejection instead of surfacing a user-visible runtime error.
    void Promise.resolve(unlisten()).catch(() => undefined);
  } catch {
    // The listener is already absent; cleanup has achieved its postcondition.
  }
}

export type SlashCommandOption = {
  trigger: string;
  description: string;
  modId: string;
  modName: string;
};

type ChatComposerProps = {
  activeStreamId: string | null;
  attachments: ChatComposerAttachment[];
  automatedWebGroundingEnabled: boolean;
  draft: string;
  dynamicRoutingEnabled: boolean;
  hasRouteModel: boolean;
  hasSelectedAgent: boolean;
  isQueueExecuting: boolean;
  isReadingAttachments: boolean;
  isSavingDynamicRoutingOverride: boolean;
  isSavingWebGroundingOverride: boolean;
  isSendMenuOpen: boolean;
  isSending: boolean;
  localModelIsHydrating: boolean;
  queuedMessageCount: number;
  routingIndicator?: ReactNode;
  selectedAgentName: string | null;
  sessionId: string;
  slashCommands?: SlashCommandOption[];
  canSubmitWhileLocalModelHydrating?: (message: string) => boolean;
  onAttachmentDrop: (dropId: string) => void | Promise<void>;
  onAttachmentRequest: () => void | Promise<void>;
  onCloseSendMenu: () => void;
  onCompactSession: () => Promise<void>;
  onDynamicRoutingToggle: () => void | Promise<void>;
  onDraftChange: (draft: string) => void;
  onExecuteQueuedMessages: () => void | Promise<void>;
  onQueueMessage: (message: string) => Promise<ChatSubmissionOutcome>;
  onRemoveAttachment: (index: number) => void;
  onStopGeneration: () => void;
  onSubmitMessage: (message: string) => Promise<ChatSubmissionOutcome>;
  onToggleSendMenu: () => void;
  onWebGroundingToggle: () => void | Promise<void>;
  onSteerNow: (message: string) => Promise<ChatSubmissionOutcome>;
};

export const ChatComposer = memo(function ChatComposer({
  activeStreamId,
  attachments,
  automatedWebGroundingEnabled,
  draft,
  dynamicRoutingEnabled,
  hasRouteModel,
  hasSelectedAgent,
  isQueueExecuting,
  isReadingAttachments,
  isSavingDynamicRoutingOverride,
  isSavingWebGroundingOverride,
  isSendMenuOpen,
  isSending,
  localModelIsHydrating,
  queuedMessageCount,
  routingIndicator,
  selectedAgentName,
  sessionId,
  slashCommands = [],
  canSubmitWhileLocalModelHydrating,
  onAttachmentDrop,
  onAttachmentRequest,
  onCloseSendMenu,
  onCompactSession,
  onDynamicRoutingToggle,
  onDraftChange,
  onExecuteQueuedMessages,
  onQueueMessage,
  onRemoveAttachment,
  onStopGeneration,
  onSubmitMessage,
  onToggleSendMenu,
  onWebGroundingToggle,
  onSteerNow,
}: ChatComposerProps) {
  const { t } = useI18n();
  const message = draft;
  const [isSlashOpen, setIsSlashOpen] = useState(false);
  const [slashFocusIndex, setSlashFocusIndex] = useState(0);
  const [isVoiceCapturing, setIsVoiceCapturing] = useState(false);
  const [isVoiceStarting, setIsVoiceStarting] = useState(false);
  const [isFileDragOver, setIsFileDragOver] = useState(false);
  const [voiceErrorKey, setVoiceErrorKey] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const dropTargetRef = useRef<HTMLFormElement>(null);
  const activeVoiceCaptureRef = useRef<string | null>(null);
  const pendingVoiceEventRef = useRef<VoiceStreamEvent | null>(null);
  const voiceStartPendingRef = useRef(false);
  const voiceDraftAnchorRef = useRef("");
  const voiceUnlistenRef = useRef<null | (() => void)>(null);
  const composerMountedRef = useRef(true);
  const submissionInFlightRef = useRef(false);
  const trimmedMessage = message.trim();
  const hasMessage = trimmedMessage.length > 0;
  const hasSubmissionContent = hasMessage || attachments.length > 0;
  const hydrationBlocksCurrentMessage =
    localModelIsHydrating &&
    !canSubmitWhileLocalModelHydrating?.(trimmedMessage);
  const slashQuery = message.startsWith("/") && !/[\s\n]/.test(message) ? message : "";
  const filteredSlashCommands = useMemo(() => {
    if (!slashQuery) {
      return slashCommands;
    }
    return slashCommands.filter((command) =>
      command.trigger.toLowerCase().startsWith(slashQuery.toLowerCase()),
    );
  }, [slashCommands, slashQuery]);
  const shouldShowSlashCommands = isSlashOpen && message.startsWith("/") && !/[\s\n]/.test(message);
  const isSendDisabled =
    !hasSubmissionContent ||
    !hasSelectedAgent ||
    isSending ||
    isReadingAttachments ||
    isQueueExecuting ||
    !hasRouteModel ||
    hydrationBlocksCurrentMessage;
  const isMenuButtonDisabled =
    !hasSubmissionContent ||
    !hasSelectedAgent ||
    isQueueExecuting ||
    !hasRouteModel ||
    hydrationBlocksCurrentMessage;
  const shouldShowSendMenu = isSendMenuOpen && !isMenuButtonDisabled;
  // The primary button's look tracks its actual actionability: a quiet
  // outline — matching the Memory chip — when there's nothing to do, and
  // an accent fill once sending (or stopping a live stream) is on offer.
  const isPrimaryDisabled = isSending ? !activeStreamId : isSendDisabled;

  // Accepted actions remount the composer to reset local affordances such as
  // textarea height and slash-menu state. The draft itself is controlled by
  // the parent and must be cleared explicitly there.
  useEffect(() => {
    textareaRef.current?.focus({ preventScroll: true });
    composerMountedRef.current = true;
    return () => {
      composerMountedRef.current = false;
      releaseEventListener(voiceUnlistenRef.current);
      voiceUnlistenRef.current = null;
      if (activeVoiceCaptureRef.current) {
        activeVoiceCaptureRef.current = null;
        void invoke<VoiceCaptureStatus>("stop_voice_capture").catch(() => undefined);
      }
    };
  }, []);

  useNativeComposerDrop({ disabled: isReadingAttachments, onDrop: onAttachmentDrop,
    setActive: setIsFileDragOver, targetRef: dropTargetRef });

  function resetTextareaHeight() {
    if (textareaRef.current) {
      textareaRef.current.style.height = "";
    }
  }

  async function submitDraft() {
    if (submissionInFlightRef.current) return;
    submissionInFlightRef.current = true;
    const submittedMessage = message;
    try {
      if (activeVoiceCaptureRef.current) await stopVoiceCapture(false);
      if (isCompactCommand(message)) {
        await onCompactSession();
        onDraftChange("");
        resetTextareaHeight();
        return;
      }
      const outcome = await onSubmitMessage(submittedMessage);
      if (outcome.accepted) {
        onDraftChange("");
        resetTextareaHeight();
      } else {
        window.requestAnimationFrame(() => textareaRef.current?.focus({ preventScroll: true }));
      }
    } catch {
      window.requestAnimationFrame(() => textareaRef.current?.focus({ preventScroll: true }));
    } finally {
      submissionInFlightRef.current = false;
    }
  }

  async function submitAlternateDraft(
    submit: (value: string) => Promise<ChatSubmissionOutcome>,
  ) {
    if (submissionInFlightRef.current) return;
    submissionInFlightRef.current = true;
    const submittedMessage = message;
    try {
      if (activeVoiceCaptureRef.current) await stopVoiceCapture(false);
      const outcome = await submit(submittedMessage);
      if (outcome.accepted) {
        onDraftChange("");
        resetTextareaHeight();
      } else {
        window.requestAnimationFrame(() => textareaRef.current?.focus({ preventScroll: true }));
      }
    } catch {
      window.requestAnimationFrame(() => textareaRef.current?.focus({ preventScroll: true }));
    } finally {
      submissionInFlightRef.current = false;
    }
  }

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (isSendDisabled) {
      return;
    }
    void submitDraft();
  }

  function applySlashCommand(trigger: string) {
    onDraftChange(`${trigger} `);
    setIsSlashOpen(false);
    window.requestAnimationFrame(() => textareaRef.current?.focus({ preventScroll: true }));
  }

  function handleMessageChange(value: string) {
    if (activeVoiceCaptureRef.current) {
      void stopVoiceCapture(false);
    }
    onDraftChange(value);
    if (value.startsWith("/") && !/[\s\n]/.test(value)) {
      setIsSlashOpen(true);
      setSlashFocusIndex(0);
      return;
    }
    setIsSlashOpen(false);
  }

  function applyVoiceStreamEvent(payload: VoiceStreamEvent) {
    const captureId = payload.capture_id ?? payload.captureId ?? "";
    if (!captureId || captureId !== activeVoiceCaptureRef.current) {
      if (captureId && voiceStartPendingRef.current && !activeVoiceCaptureRef.current) {
        pendingVoiceEventRef.current = payload;
      }
      return;
    }

    const errorCode = payload.error_code ?? payload.errorCode;
    if (errorCode) {
      activeVoiceCaptureRef.current = null;
      setIsVoiceCapturing(false);
      setIsVoiceStarting(false);
      setVoiceErrorKey(voiceInputErrorKey(errorCode));
      void invoke<VoiceCaptureStatus>("stop_voice_capture").catch(() => undefined);
      return;
    }

    const transcript = typeof payload.text === "string" ? payload.text.trim() : "";
    if (transcript) {
      onDraftChange(appendVoiceTranscript(voiceDraftAnchorRef.current, transcript));
      window.requestAnimationFrame(() => textareaRef.current?.focus({ preventScroll: true }));
    }
    if (payload.is_final ?? payload.isFinal ?? false) {
      activeVoiceCaptureRef.current = null;
      setIsVoiceCapturing(false);
      setIsVoiceStarting(false);
      void invoke<VoiceCaptureStatus>("stop_voice_capture").catch(() => undefined);
    }
  }

  async function ensureVoiceListener() {
    if (voiceUnlistenRef.current) {
      return;
    }
    voiceUnlistenRef.current = await listen<VoiceStreamEvent>(
      "oomu://voice-stream",
      (event) => applyVoiceStreamEvent(event.payload),
    );
  }

  async function startVoiceCapture() {
    if (voiceStartPendingRef.current || activeVoiceCaptureRef.current) {
      return;
    }
    voiceStartPendingRef.current = true;
    pendingVoiceEventRef.current = null;
    setVoiceErrorKey(null);
    setIsVoiceStarting(true);
    voiceDraftAnchorRef.current = message;
    try {
      await ensureVoiceListener();
      const status = await invoke<VoiceCaptureStatus>("start_voice_capture");
      const captureId = status.capture_id ?? status.captureId ?? "";
      if (!status.active || !captureId) {
        throw new Error("voice_input_start_failed");
      }
      activeVoiceCaptureRef.current = captureId;
      voiceStartPendingRef.current = false;
      if (composerMountedRef.current) {
        setIsVoiceCapturing(true);
      }
      const pendingEvent = pendingVoiceEventRef.current;
      pendingVoiceEventRef.current = null;
      if (pendingEvent) {
        applyVoiceStreamEvent(pendingEvent);
      }
    } catch (error) {
      activeVoiceCaptureRef.current = null;
      voiceStartPendingRef.current = false;
      pendingVoiceEventRef.current = null;
      if (composerMountedRef.current) {
        setIsVoiceCapturing(false);
        setVoiceErrorKey(voiceInputErrorKey(invokeErrorCode(error)));
      }
    } finally {
      if (composerMountedRef.current) {
        setIsVoiceStarting(false);
      }
    }
  }

  async function stopVoiceCapture(surfaceError = true) {
    const hadActiveCapture = Boolean(activeVoiceCaptureRef.current);
    activeVoiceCaptureRef.current = null;
    voiceStartPendingRef.current = false;
    pendingVoiceEventRef.current = null;
    if (composerMountedRef.current) {
      setIsVoiceCapturing(false);
      setIsVoiceStarting(false);
    }
    if (!hadActiveCapture) {
      return;
    }
    try {
      await invoke<VoiceCaptureStatus>("stop_voice_capture");
    } catch {
      if (surfaceError && composerMountedRef.current) {
        setVoiceErrorKey("chat.voice_input_error");
      }
    }
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (shouldShowSlashCommands) {
      if (event.key === "ArrowDown" && filteredSlashCommands.length > 0) {
        event.preventDefault();
        setSlashFocusIndex((current) => (current + 1) % filteredSlashCommands.length);
        return;
      }
      if (event.key === "ArrowUp" && filteredSlashCommands.length > 0) {
        event.preventDefault();
        setSlashFocusIndex(
          (current) => (current - 1 + filteredSlashCommands.length) % filteredSlashCommands.length,
        );
        return;
      }
      if (event.key === "Enter") {
        event.preventDefault();
        const selected = filteredSlashCommands[slashFocusIndex];
        if (selected) {
          applySlashCommand(selected.trigger);
        }
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setIsSlashOpen(false);
        return;
      }
    }
    if (event.key !== "Enter" || event.shiftKey) {
      return;
    }
    event.preventDefault();
    if (
      !hasSubmissionContent ||
      !hasSelectedAgent ||
      !hasRouteModel ||
      hydrationBlocksCurrentMessage ||
      isQueueExecuting
    ) {
      return;
    }
    if (isSending) {
      if (!isQueueExecuting) {
        void submitAlternateDraft(onQueueMessage);
      }
      return;
    }
    void submitDraft();
  }

  return (
    <form className="relative shrink-0 border-t border-[var(--border-strong)] bg-[var(--background)] p-4 lg:px-8" data-chat-drop-target data-chat-session-id={sessionId} onSubmit={handleSubmit} ref={dropTargetRef}>
      {isFileDragOver && (
        <div
          aria-live="polite"
          className="pointer-events-none absolute inset-2 z-40 flex items-center justify-center rounded-[var(--radius-lg)] border-2 border-[var(--accent)] bg-[var(--background)]/95"
          role="status"
        >
          <span className="inline-flex items-center gap-2 rounded-full bg-[var(--accent-background)] px-4 py-2 text-sm font-semibold text-[var(--accent)]">
            <AttachmentIcon />
            {t("chat.drop_to_attach")}
          </span>
        </div>
      )}
      <div className="flex w-full flex-col gap-3">
        {attachments.length > 0 && (
          <div className="flex flex-wrap gap-2">
            {attachments.map((attachment, index) => (
              <button
                aria-label={t("chat.remove_attachment", { name: attachment.name })}
                className="rounded-[var(--radius-sm)] inline-flex items-center gap-2 border border-[var(--border-strong)] bg-[var(--background)] px-2.5 py-1 text-xs font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
                key={`${attachment.name}-${attachment.byte_count}-${index}`}
                onClick={() => onRemoveAttachment(index)}
                type="button"
              >
                <AttachmentIcon />
                <span className="max-w-[12rem] truncate">{attachment.name}</span>
                <span aria-hidden="true" className="text-[var(--foreground-subtle)]">×</span>
              </button>
            ))}
          </div>
        )}

        {queuedMessageCount > 0 && (
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-[var(--radius-base)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-3 py-2">
            <span className="text-xs font-semibold text-[var(--foreground-muted)]">
              {queuedMessageCount === 1
                ? t("chat.queued_one")
                : t("chat.queued_many", { count: queuedMessageCount })}
            </span>
            <button
              className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-1.5 text-xs font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-40"
              disabled={isQueueExecuting || isSending || hydrationBlocksCurrentMessage}
              onClick={() => void onExecuteQueuedMessages()}
              type="button"
            >
              {isQueueExecuting ? t("chat.running") : t("chat.run_queue")}
            </button>
          </div>
        )}

        {hydrationBlocksCurrentMessage && (
          <div
            aria-live="polite"
            className="grid gap-2 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-3 py-2"
            role="status"
          >
            <div className="flex items-center gap-2 text-xs font-semibold text-[var(--foreground)]">
              <span className="h-3 w-3 shrink-0 animate-spin rounded-full border-2 border-[var(--accent)] border-t-transparent" />
              <span>{t("chat.hydrating")}</span>
            </div>
            <div className="h-1.5 overflow-hidden rounded-full bg-[var(--border-soft)]">
              <div className="h-full w-2/5 animate-pulse rounded-full bg-[var(--accent)]" />
            </div>
          </div>
        )}

        {/* Composer: the textarea is the hero on its own full-width row; the
            attach / search / auto-route controls sit on a quiet utility row
            beneath it so they never crowd the input. */}
        <div
          className={`relative flex flex-col gap-2 rounded-[var(--radius-lg)] border bg-[var(--background)] p-2 transition-colors focus-within:border-[var(--accent)] ${
            isFileDragOver
              ? "border-[var(--accent)]"
              : "border-[var(--border-strong)]"
          }`}
        >
          {routingIndicator && (
            <div className="flex items-center justify-start px-1">
              {routingIndicator}
            </div>
          )}
          <div className="relative">
            {shouldShowSlashCommands && (
              <div
                aria-label={t("chat.slash_commands.aria_label")}
                className="absolute bottom-full left-0 right-0 z-30 mb-2 max-h-72 overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] shadow-[0_18px_40px_rgba(15,23,42,0.16)]"
                role="listbox"
              >
                {filteredSlashCommands.length > 0 ? (
                  <div className="custom-scrollbar max-h-72 overflow-y-auto p-1">
                    {filteredSlashCommands.map((command, index) => {
                      const isFocused = index === slashFocusIndex;
                      return (
                        <button
                          aria-selected={isFocused}
                          className={`grid w-full grid-cols-[minmax(5rem,auto)_1fr] items-start gap-3 rounded-[var(--radius-sm)] px-3 py-2 text-left transition-colors ${
                            isFocused
                              ? "bg-[var(--accent-background)] text-[var(--foreground)]"
                              : "text-[var(--foreground)] hover:bg-[var(--fill-hover)]"
                          }`}
                          key={`${command.modId}-${command.trigger}`}
                          onMouseDown={(event) => {
                            event.preventDefault();
                            applySlashCommand(command.trigger);
                          }}
                          role="option"
                          type="button"
                        >
                          <span className="text-xs font-semibold uppercase text-[var(--foreground-muted)]">
                            {t("chat.slash_commands.prefix_label")}
                          </span>
                          <span className="min-w-0">
                            <span className="block text-sm font-semibold text-[var(--foreground)]">
                              {command.trigger}
                            </span>
                            <span className="mt-0.5 block break-words text-xs leading-5 text-[var(--foreground-muted)]">
                              {command.description || command.modName}
                            </span>
                          </span>
                        </button>
                      );
                    })}
                  </div>
                ) : (
                  <p className="px-3 py-3 text-sm font-medium text-[var(--foreground-muted)]">
                    {t("chat.slash_commands.no_matches")}
                  </p>
                )}
              </div>
            )}
            <textarea
              className="max-h-40 min-h-[44px] w-full resize-none bg-transparent px-2 py-2 text-sm font-medium leading-6 text-[var(--foreground)] outline-none placeholder:text-[var(--foreground-subtle)]" id="oomu-chat-composer"
              onChange={(event) => handleMessageChange(event.target.value)}
              onKeyDown={handleKeyDown}
              placeholder={t("chat.message_placeholder", { name: selectedAgentName ?? t("chat.an_agent") })}
              ref={textareaRef}
              value={message}
            />
          </div>
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-1">
              <button
                aria-label={t("chat.attachment")}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[var(--radius-sm)] text-[var(--foreground-muted)] transition-colors hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
                disabled={isReadingAttachments}
                onClick={() => void onAttachmentRequest()}
                type="button"
              >
                <AttachmentIcon />
              </button>
              <button
                aria-label={t("chat.search")}
                aria-pressed={automatedWebGroundingEnabled}
                className={`relative flex h-9 w-9 shrink-0 items-center justify-center rounded-[var(--radius-sm)] transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
                  automatedWebGroundingEnabled
                    ? "text-[var(--accent)] hover:bg-[var(--fill-hover)]"
                    : "text-[var(--foreground-muted)] hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
                }`}
                disabled={!hasSelectedAgent || isSavingWebGroundingOverride} id="oomu-chat-search"
                onClick={() => void onWebGroundingToggle()}
                title={t("chat.search_tooltip")}
                type="button"
              >
                <GlobeIcon />
                {automatedWebGroundingEnabled && (
                  <span aria-hidden="true" className="absolute bottom-1 right-1 h-1.5 w-1.5 rounded-full bg-[var(--accent)]" />
                )}
              </button>
              <button
                aria-label={t("chat.auto_route")}
                aria-pressed={dynamicRoutingEnabled}
                className={`flex h-9 shrink-0 items-center gap-1.5 rounded-[var(--radius-sm)] px-2.5 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
                  dynamicRoutingEnabled
                    ? "text-[var(--accent)] hover:bg-[var(--fill-hover)]"
                    : "text-[var(--foreground-muted)] hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
                }`}
                data-oomu-routing-control="auto-route" disabled={!hasSelectedAgent || isSavingDynamicRoutingOverride} id="oomu-auto-route-toggle"
                onClick={() => void onDynamicRoutingToggle()}
                title={
                  dynamicRoutingEnabled
                    ? t("chat.auto_route_tooltip_on")
                    : t("chat.auto_route_tooltip_off")
                }
                type="button"
              >
                <DynamicRoutingIcon />
                <span>{t("chat.auto_route")}</span>
                {dynamicRoutingEnabled && (
                  <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-[var(--accent)]" />
                )}
              </button>
            </div>
            <div className="flex items-center gap-2">
              <button
                aria-busy={isVoiceStarting}
                aria-label={t("chat.voice_input")}
                aria-pressed={isVoiceCapturing}
                className={`relative flex h-8 w-8 shrink-0 items-center justify-center rounded-[var(--radius-sm)] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--background)] disabled:cursor-not-allowed disabled:opacity-40 ${
                  isVoiceCapturing
                    ? "border border-[var(--destructive)] bg-[var(--destructive-background)] text-[var(--destructive)]"
                    : "border border-[var(--border-strong)] text-[var(--foreground-muted)] hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
                }`}
                disabled={
                  isVoiceStarting ||
                  isSending ||
                  isQueueExecuting ||
                  !hasSelectedAgent ||
                  !hasRouteModel
                }
                onClick={() =>
                  void (isVoiceCapturing ? stopVoiceCapture() : startVoiceCapture())
                }
                title={
                  isVoiceCapturing ? t("chat.voice_input_stop") : t("chat.voice_input")
                }
                type="button"
              >
                {isVoiceCapturing && (
                  <span
                    aria-hidden="true"
                    className="absolute h-6 w-6 animate-ping rounded-full bg-[var(--destructive-background)]"
                  />
                )}
                {isVoiceStarting ? (
                  <span
                    aria-hidden="true"
                    className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-t-transparent"
                  />
                ) : (
                  <MicrophoneIcon />
                )}
              </button>
              <div className="relative flex h-8 shrink-0">
                <button
                  aria-label={isSending ? t("chat.stop") : t("chat.send")}
                  className={`flex h-8 shrink-0 items-center justify-center gap-2 rounded-l-[var(--radius-sm)] rounded-r-none border px-4 text-sm font-medium transition-colors disabled:cursor-not-allowed ${
                    isPrimaryDisabled
                      ? "border-[var(--border-strong)] border-r-0 text-[var(--foreground-muted)]"
                      : "border-transparent bg-[var(--inverse-background)] text-[var(--inverse-foreground)] hover:bg-[var(--accent-hover)]"
                  }`}
                  disabled={isSending ? !activeStreamId : isSendDisabled} id={["oomu-chat-send", "oomu-chat-stop"][Number(isSending)]}
                  onClick={isSending ? onStopGeneration : undefined}
                  type={isSending ? "button" : "submit"}
                >
                  <span>{isSending ? t("chat.stop") : hydrationBlocksCurrentMessage ? t("chat.hydrating") : t("chat.send")}</span>
                  {isSending ? (
                    <span aria-hidden="true" className="h-3 w-3 bg-current" />
                  ) : (
                    <SendIcon />
                  )}
                </button>
                <button
                  aria-expanded={shouldShowSendMenu}
                  aria-haspopup="menu"
                  aria-label={t("chat.send_options")}
                  className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-l-none rounded-r-[var(--radius-sm)] border transition-colors disabled:cursor-not-allowed ${
                    isMenuButtonDisabled
                      ? "border-[var(--border-strong)] text-[var(--foreground-muted)]"
                      : "border-transparent border-l-[var(--background)] bg-[var(--inverse-background)] text-[var(--inverse-foreground)] hover:bg-[var(--accent-hover)]"
                  }`}
                  disabled={isMenuButtonDisabled}
                  onClick={onToggleSendMenu}
                  type="button"
                >
                  <ChevronDownIcon />
                </button>
                {shouldShowSendMenu && (
                  <div
                    className="absolute bottom-full right-0 z-20 mb-2 w-56 rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] shadow-[0_12px_24px_rgba(15,23,42,0.12)]"
                    role="menu"
                  >
                    <button
                      className="flex w-full items-center px-3 py-2 text-left text-sm font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--accent-background)]"
                      onClick={() => {
                        onCloseSendMenu();
                        void submitAlternateDraft(onQueueMessage);
                      }}
                      role="menuitem"
                      type="button"
                    >
                      {t("chat.queue_message")}
                    </button>
                    <button
                      className="flex w-full items-center px-3 py-2 text-left text-sm font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--accent-background)]"
                      onClick={() => {
                        onCloseSendMenu();
                        void submitAlternateDraft(onSteerNow);
                      }}
                      role="menuitem"
                      type="button"
                    >
                      {t("chat.steer")}
                    </button>
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
        {voiceErrorKey && (
          <p
            className="px-1 text-xs leading-5 text-[var(--destructive)]"
            role="alert"
          >
            {t(voiceErrorKey)}
          </p>
        )}
      </div>
    </form>
  );
});

function isCompactCommand(value: string) {
  return value.trim().toLowerCase() === "/compact";
}

function appendVoiceTranscript(existingDraft: string, transcript: string) {
  const cleanTranscript = transcript.trim();
  if (!cleanTranscript) {
    return existingDraft;
  }
  if (!existingDraft || /\s$/.test(existingDraft)) {
    return `${existingDraft}${cleanTranscript}`;
  }
  return `${existingDraft} ${cleanTranscript}`;
}

function voiceInputErrorKey(errorCode: string | undefined) {
  if (
    errorCode === "microphone_permission_denied" ||
    errorCode === "speech_permission_denied"
  ) {
    return "chat.voice_input_permission";
  }
  if (errorCode === "on_device_unavailable" || errorCode === "speech_unavailable") {
    return "chat.voice_input_unavailable";
  }
  return "chat.voice_input_error";
}

function invokeErrorCode(error: unknown) {
  if (error && typeof error === "object" && "code" in error) {
    const code = (error as { code?: unknown }).code;
    return typeof code === "string" ? code : undefined;
  }
  if (error instanceof Error) {
    return error.message.match(/\b[a-z][a-z0-9_]+\b/i)?.[0];
  }
  return undefined;
}

function AttachmentIcon() {
  return (
    <svg aria-hidden="true" className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24">
      <path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.82-2.82l8.48-8.49" />
    </svg>
  );
}

function SendIcon() {
  return (
    <svg aria-hidden="true" className="h-4 w-4" fill="none" stroke="currentColor" strokeLinejoin="round" strokeWidth="2" viewBox="0 0 24 24">
      <path d="M22 2 11 13" />
      <path d="m22 2-7 20-4-9-9-4 20-7Z" />
    </svg>
  );
}

function MicrophoneIcon() {
  return (
    <svg
      aria-hidden="true"
      className="relative h-4 w-4"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.8"
      viewBox="0 0 24 24"
    >
      <rect height="12" rx="4" width="7" x="8.5" y="2" />
      <path d="M5 10a7 7 0 0 0 14 0" />
      <path d="M12 17v5" />
      <path d="M9 22h6" />
    </svg>
  );
}

function ChevronDownIcon() {
  return (
    <svg aria-hidden="true" className="h-4 w-4" fill="none" stroke="currentColor" strokeLinecap="square" strokeLinejoin="miter" strokeWidth="2" viewBox="0 0 24 24">
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

function GlobeIcon() {
  return (
    <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeWidth="1.8" viewBox="0 0 24 24">
      <circle cx="12" cy="12" r="10" />
      <path d="M2 12h20" />
      <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10Z" />
    </svg>
  );
}

function DynamicRoutingIcon() {
  return (
    <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">
      <path d="M6 4v5a4 4 0 0 0 4 4h1" />
      <path d="M6 20v-5a4 4 0 0 1 4-4h1" />
      <path d="m13 3-2 7h4l-2 7" />
      <path d="M17 13h1a4 4 0 0 0 4-4V4" />
      <path d="M17 11h1a4 4 0 0 1 4 4v5" />
    </svg>
  );
}
