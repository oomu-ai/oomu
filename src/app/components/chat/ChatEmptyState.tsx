"use client";

import { useId, useState, useSyncExternalStore } from "react";
import { useI18n } from "@/context/I18nContext";
import {
  type DecisionBriefCompletionState,
  dismissFirstRunChatWelcome,
  firstRunChatWelcomeIsDismissed,
  subscribeToFirstRunChatWelcome,
} from "./firstRunWelcomeState";

export type ChatStarterAction =
  | "weekly_brief"
  | "summarize_folder"
  | "help_with_email";

export type ChatStarterHandler = (
  action: ChatStarterAction,
) => boolean | void | Promise<boolean | void>;

const starterActions: ReadonlyArray<{
  id: ChatStarterAction;
  titleKey: string;
  descriptionKey: string;
}> = [
  {
    id: "weekly_brief",
    titleKey: "chat.welcome.actions.weekly_brief.title",
    descriptionKey: "chat.welcome.actions.weekly_brief.description",
  },
  {
    id: "summarize_folder",
    titleKey: "chat.welcome.actions.summarize_folder.title",
    descriptionKey: "chat.welcome.actions.summarize_folder.description",
  },
  {
    id: "help_with_email",
    titleKey: "chat.welcome.actions.help_with_email.title",
    descriptionKey: "chat.welcome.actions.help_with_email.description",
  },
];

function FirstRunChatWelcome({
  actionError,
  onAction,
  onDismiss,
  pendingAction,
}: {
  actionError: ChatStarterAction | null;
  onAction: (action: ChatStarterAction) => void;
  onDismiss: () => void;
  pendingAction: ChatStarterAction | null;
}) {
  const { t } = useI18n();
  const titleId = useId();

  return (
    <section
      aria-labelledby={titleId}
      className="w-full max-w-3xl self-start rounded-[var(--radius-lg)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-5 shadow-[var(--shadow-card)]"
    >
      <h2 className="text-xl font-semibold tracking-[-0.02em] text-[var(--foreground)]" id={titleId}>
        {t("chat.welcome.title")}
      </h2>
      <p className="mt-2 max-w-2xl text-sm leading-6 text-[var(--foreground-muted)]">
        {t("chat.welcome.subtitle")}
      </p>
      <p className="mt-2 max-w-2xl text-xs leading-5 text-[var(--foreground-muted)]">
        {t("common.results_location")}
      </p>

      <div className="mt-5 grid gap-2 sm:grid-cols-3">
        {starterActions.map((action) => (
          <button
            className={`group flex min-h-32 flex-col rounded-[var(--radius-md)] border bg-[var(--background)] p-4 text-left transition-[border-color,transform,box-shadow] hover:-translate-y-0.5 hover:border-[var(--border-strong)] hover:shadow-[var(--shadow-card)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] disabled:cursor-wait disabled:opacity-60 ${
              action.id === "weekly_brief"
                ? "border-[var(--accent)]/40"
                : "border-[var(--border-soft)]"
            }`}
            disabled={pendingAction !== null}
            key={action.id}
            onClick={() => onAction(action.id)}
            type="button"
          >
            <span className="text-sm font-semibold text-[var(--foreground)]">
              {t(action.titleKey)}
            </span>
            <span className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">
              {t(action.descriptionKey)}
            </span>
            <span aria-hidden="true" className="mt-auto pt-3 text-[var(--accent)] transition-transform group-hover:translate-x-0.5">
              <svg className="h-4 w-4" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" viewBox="0 0 24 24">
                <path d="M5 12h14M14 7l5 5-5 5" />
              </svg>
            </span>
          </button>
        ))}
      </div>

      {actionError ? (
        <p className="mt-3 text-xs text-[var(--warning)]" role="alert">
          {t(
            actionError === "summarize_folder"
              ? "chat.welcome.folder_error"
              : "chat.welcome.action_error",
          )}
        </p>
      ) : null}

      <button
        className="mt-4 rounded-[var(--radius-sm)] px-2 py-1 text-xs font-semibold text-[var(--foreground-muted)] transition-colors hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)] disabled:cursor-wait disabled:opacity-60"
        disabled={pendingAction !== null}
        onClick={onDismiss}
        type="button"
      >
        {t("chat.welcome.dismiss")}
      </button>
    </section>
  );
}

export function ChatEmptyState({
  agentDescription,
  agentName,
  decisionBriefCompletion = "incomplete",
  onStarterAction,
  sessionCount,
  sessionsLoaded,
  transcriptLoaded = true,
  transcriptEmpty,
}: {
  agentDescription: string | null;
  agentName: string | null;
  decisionBriefCompletion?: DecisionBriefCompletionState;
  onStarterAction?: ChatStarterHandler;
  sessionCount: number;
  sessionsLoaded: boolean;
  transcriptLoaded?: boolean;
  transcriptEmpty: boolean;
}) {
  const { t } = useI18n();
  const persistedDismissal = useSyncExternalStore(
    subscribeToFirstRunChatWelcome,
    firstRunChatWelcomeIsDismissed,
    () => true,
  );
  const [dismissedForRender, setDismissedForRender] = useState(false);
  const [pendingAction, setPendingAction] = useState<ChatStarterAction | null>(null);
  const [actionError, setActionError] = useState<ChatStarterAction | null>(null);
  const dismissed = persistedDismissal || dismissedForRender;

  if (!transcriptEmpty) return null;
  if (!transcriptLoaded) return null;
  if (decisionBriefCompletion === "checking") return null;

  const dismiss = () => {
    setDismissedForRender(true);
    dismissFirstRunChatWelcome();
  };
  const handleAction = async (action: ChatStarterAction) => {
    if (!onStarterAction || pendingAction) return;
    setPendingAction(action);
    setActionError(null);
    try {
      const started = await onStarterAction(action);
      if (started !== false) dismiss();
    } catch {
      setActionError(action);
    } finally {
      setPendingAction(null);
    }
  };
  const showWelcome =
    sessionsLoaded &&
    sessionCount <= 1 &&
    decisionBriefCompletion === "incomplete" &&
    !dismissed &&
    Boolean(onStarterAction);

  if (showWelcome) {
    return (
      <FirstRunChatWelcome
        actionError={actionError}
        onAction={handleAction}
        onDismiss={dismiss}
        pendingAction={pendingAction}
      />
    );
  }

  return (
    <div className="max-w-3xl self-start rounded-[var(--radius-lg)] border border-[var(--border-soft)] bg-[var(--accent-background)] px-4 py-3">
      <p className="text-xs font-semibold text-[var(--foreground-muted)]">
        {t("common.oomu")}
      </p>
      <p className="mt-2 text-sm leading-6 text-[var(--foreground)]">
        {agentName ? (
          <>
            {t("chat.agent_ready", { name: agentName })} {agentDescription}
          </>
        ) : (
          t("chat.pick_agent")
        )}
      </p>
      {onStarterAction ? (
        <button
          className="mt-3 rounded-[var(--radius-sm)] px-2 py-1 text-xs font-semibold text-[var(--accent)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-wait disabled:opacity-60"
          disabled={pendingAction !== null}
          onClick={() => handleAction("weekly_brief")}
          type="button"
        >
          {t("chat.welcome.actions.weekly_brief.title")}
        </button>
      ) : null}
      {actionError === "weekly_brief" ? (
        <p className="mt-2 text-xs text-[var(--warning)]" role="alert">
          {t("chat.welcome.action_error")}
        </p>
      ) : null}
    </div>
  );
}
