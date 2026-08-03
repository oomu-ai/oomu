"use client";

import { useI18n } from "@/context/I18nContext";
import type { ChatSession } from "@/lib/chatSessions";
import type { MutableRefObject } from "react";
import { ChatAgentSelector } from "./ChatAgentSelector";
import {
  NewChatIcon,
  PencilIcon,
  SlidersIcon,
  SplitPaneIcon,
  StatusIndicator,
} from "./ChatScreenIcons";
import { DocumentsNavIcon, TasksNavIcon } from "./ChatNavigationIcons";
import { ProjectChatScopeBanner } from "./ProjectChatScopeBanner";
import { formatChatSessionTime } from "./sessionPresentation";

type ChatAgentOption = { id: string; name: string };

const headerControlClass =
  "inline-flex items-center gap-2 border border-transparent px-3 py-1.5 text-sm font-semibold text-[var(--foreground)]";
const interactiveHeaderControlClass =
  `${headerControlClass} transition-colors hover:bg-[var(--accent-background)]`;

type SessionItemProps = {
  agent?: ChatAgentOption;
  editingTitle: string;
  isActive: boolean;
  isEditing: boolean;
  isRenaming: boolean;
  isWorking: boolean;
  onAbortRename: () => void;
  onBeginRename: (session: ChatSession) => void;
  onCommitRename: (session: ChatSession) => void | Promise<void>;
  onDelete: (sessionId: string) => void | Promise<unknown>;
  onEditingTitleChange: (title: string) => void;
  onSelect: (sessionId: string) => void;
  session: ChatSession;
  skipRenameCommitRef: MutableRefObject<boolean>;
};

function ChatSessionListItem({
  agent,
  editingTitle,
  isActive,
  isEditing,
  isRenaming,
  isWorking,
  onAbortRename,
  onBeginRename,
  onCommitRename,
  onDelete,
  onEditingTitleChange,
  onSelect,
  session,
  skipRenameCommitRef,
}: SessionItemProps) {
  const { t } = useI18n();
  const title = session.title === "New Session" ? t("chat.new_session") : session.title;
  const agentName = agent?.name ?? t("chat.agent_fallback");
  return (
    <div
      className={`group relative cursor-default rounded-[var(--radius-sm)] px-3 py-2 transition-colors ${isActive ? "bg-[var(--fill-selected)]" : "hover:bg-[var(--fill-hover)]"}`}
      id={`oomu-chat-session-${session.id}`}
      onClick={() => onSelect(session.id)}
      onKeyDown={(event) => {
        if (event.key === "Enter") onSelect(session.id);
      }}
      role="button"
      tabIndex={0}
    >
      {isEditing ? (
        <input
          aria-label={t("chat.rename_session", { title })}
          autoFocus
          className="mb-0.5 w-full rounded-[var(--radius-sm)] border border-[var(--accent)] bg-[var(--background)] px-2 py-1 text-sm font-medium text-[var(--foreground)] outline-none"
          disabled={isRenaming}
          onBlur={() => {
            if (skipRenameCommitRef.current) {
              skipRenameCommitRef.current = false;
              return;
            }
            void onCommitRename(session);
          }}
          onChange={(event) => onEditingTitleChange(event.target.value)}
          onClick={(event) => event.stopPropagation()}
          onFocus={(event) => event.currentTarget.select()}
          onKeyDown={(event) => {
            event.stopPropagation();
            if (event.key === "Enter") {
              event.preventDefault();
              void onCommitRename(session);
            } else if (event.key === "Escape") {
              event.preventDefault();
              onAbortRename();
            }
          }}
          value={editingTitle}
        />
      ) : (
        <div className="flex min-w-0 items-center gap-2 pr-14">
          <p className="min-w-0 truncate text-sm font-medium text-[var(--foreground)]">{title}</p>
          {isWorking ? (
            <span
              aria-label={t("chat.thinking_named", { name: agentName })}
              className="h-3 w-3 shrink-0 animate-spin rounded-full border-2 border-[var(--accent)] border-t-transparent"
              role="status"
            />
          ) : session.unreadCompletion && !isActive ? (
            <span
              aria-label={t("chat.background_completion_unread")}
              className="h-2 w-2 shrink-0 rounded-full bg-[var(--accent)] shadow-[0_0_0_3px_var(--accent-background)]"
              role="status"
            />
          ) : null}
        </div>
      )}
      <p className="mt-0.5 truncate text-xs text-[var(--foreground-muted)]">
        {agentName}{formatChatSessionTime(session.updatedAtMs) ? ` · ${formatChatSessionTime(session.updatedAtMs)}` : ""}
      </p>
      {!isEditing ? (
        <button
          aria-label={t("chat.rename_session", { title })}
          className="absolute right-8 top-1.5 hidden h-6 w-6 items-center justify-center rounded-full text-[var(--foreground-subtle)] transition-colors hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)] group-hover:flex"
          onClick={(event) => {
            event.stopPropagation();
            onBeginRename(session);
          }}
          type="button"
        >
          <PencilIcon />
        </button>
      ) : null}
      {!isEditing ? (
        <button
          aria-label={t("chat.delete_session", { title })}
          className="absolute right-1.5 top-1.5 hidden h-6 w-6 items-center justify-center rounded-full text-[var(--foreground-subtle)] transition-colors hover:bg-[var(--fill-hover)] hover:text-[var(--destructive)] group-hover:flex"
          onClick={(event) => {
            event.stopPropagation();
            void onDelete(session.id);
          }}
          type="button"
        >
          <svg aria-hidden="true" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="2" viewBox="0 0 24 24">
            <path d="M6 6l12 12M18 6 6 18" />
          </svg>
        </button>
      ) : null}
    </div>
  );
}

type ChatSessionsSidebarProps = {
  activeSessionId: string;
  agentById: ReadonlyMap<string, ChatAgentOption>;
  canCreateSession: boolean;
  editingSessionId: string | null;
  editingSessionTitle: string;
  isProcessingForSession: (sessionId: string) => boolean;
  isRenamingSession: boolean;
  onAbortRename: () => void;
  onBeginRename: (session: ChatSession) => void;
  onCommitRename: (session: ChatSession) => void | Promise<void>;
  onCreateSession: () => void | Promise<void>;
  onDeleteSession: (sessionId: string) => void | Promise<unknown>;
  onEditingTitleChange: (title: string) => void;
  onSelectSession: (sessionId: string) => void;
  onStartGlobalChat?: () => void;
  projectId: string | null;
  sessions: ChatSession[];
  skipRenameCommitRef: MutableRefObject<boolean>;
  width: number;
};

export function ChatSessionsSidebar({
  activeSessionId,
  agentById,
  canCreateSession,
  editingSessionId,
  editingSessionTitle,
  isProcessingForSession,
  isRenamingSession,
  onAbortRename,
  onBeginRename,
  onCommitRename,
  onCreateSession,
  onDeleteSession,
  onEditingTitleChange,
  onSelectSession,
  onStartGlobalChat,
  projectId,
  sessions,
  skipRenameCommitRef,
  width,
}: ChatSessionsSidebarProps) {
  const { t } = useI18n();
  return (
    <aside className="flex shrink-0 flex-col" style={{ width }}>
      <div className="flex shrink-0 items-center justify-between px-4 pb-2 pt-4">
        <h2 className="text-sm font-semibold text-[var(--foreground)]">{t("chat.chats")}</h2>
        <button
          aria-label={projectId ? t("chat.project_scope.new_project_chat") : t("chat.new_chat")}
          className="flex h-7 w-7 items-center justify-center rounded-[var(--radius-sm)] text-[var(--accent)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-40"
          disabled={!canCreateSession}
          id="oomu-chat-new"
          onClick={() => void onCreateSession()}
          title={projectId ? t("chat.project_scope.new_project_chat") : t("chat.new_chat")}
          type="button"
        >
          <NewChatIcon />
        </button>
      </div>
      {projectId && onStartGlobalChat ? (
        <ProjectChatScopeBanner onStartGlobalChat={onStartGlobalChat} t={t} />
      ) : null}
      <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {sessions.length === 0 ? (
          <p className="px-3 py-2 text-xs leading-5 text-[var(--foreground-muted)]">
            {t("chat.no_conversations")}
          </p>
        ) : (
          <div className="flex flex-col gap-0.5">
            {sessions.map((session) => {
              const isActive = session.id === activeSessionId;
              return (
                <ChatSessionListItem
                  agent={agentById.get(session.agentId)}
                  editingTitle={editingSessionTitle}
                  isActive={isActive}
                  isEditing={editingSessionId === session.id}
                  isRenaming={isRenamingSession}
                  isWorking={!isActive && isProcessingForSession(session.id)}
                  key={session.id}
                  onAbortRename={onAbortRename}
                  onBeginRename={onBeginRename}
                  onCommitRename={onCommitRename}
                  onDelete={onDeleteSession}
                  onEditingTitleChange={onEditingTitleChange}
                  onSelect={onSelectSession}
                  session={session}
                  skipRenameCommitRef={skipRenameCommitRef}
                />
              );
            })}
          </div>
        )}
      </div>
    </aside>
  );
}

export function ChatWorkspaceHeader({
  activeAgentId,
  agents,
  hasSelectedAgent,
  hasSplitPanelContent,
  isDrawerOpen,
  isSplitPanelOpen,
  onAgentChange,
  onManageAgents,
  onOpenDocuments,
  onOpenTasks,
  onToggleSplit,
  onToggleTuning,
}: {
  activeAgentId: string;
  agents: readonly ChatAgentOption[];
  hasSelectedAgent: boolean;
  hasSplitPanelContent: boolean;
  isDrawerOpen: boolean;
  isSplitPanelOpen: boolean;
  onAgentChange: (agentId: string) => void | Promise<void>;
  onManageAgents?: () => void;
  onOpenDocuments?: () => void;
  onOpenTasks?: () => void;
  onToggleSplit: () => void;
  onToggleTuning: () => void;
}) {
  const { t } = useI18n();
  return (
    <header className="flex shrink-0 items-center justify-between gap-4 border-b border-[var(--border-strong)] px-5 py-3">
      {onOpenTasks || onOpenDocuments ? (
        <nav aria-label={t("common.results")} className="flex shrink-0 items-center gap-1">
          {onOpenTasks ? <button className={interactiveHeaderControlClass} onClick={onOpenTasks} type="button"><TasksNavIcon />{t("common.all_tasks")}</button> : null}
          {onOpenDocuments ? <button className={interactiveHeaderControlClass} onClick={onOpenDocuments} type="button"><DocumentsNavIcon />{t("common.documents")}</button> : null}
        </nav>
      ) : null}
      <div className="flex w-full shrink-0 items-center flex-col gap-2 sm:flex-row sm:justify-end lg:w-auto">
        <StatusIndicator available={hasSelectedAgent} className={headerControlClass} label={hasSelectedAgent ? t("chat.available") : t("chat.no_agent")} />
        <ChatAgentSelector activeAgentId={activeAgentId} agents={agents} controlClassName={interactiveHeaderControlClass} onAgentChange={onAgentChange} onManageAgents={onManageAgents} />
        <button className={`${interactiveHeaderControlClass} ${isDrawerOpen ? "bg-[var(--accent-background)]" : ""}`} id="oomu-chat-tuning" onClick={onToggleTuning} type="button">
          <SlidersIcon />{t("chat.tuning")}
        </button>
        <button
          aria-pressed={hasSplitPanelContent ? isSplitPanelOpen : false}
          className={`${hasSplitPanelContent ? interactiveHeaderControlClass : headerControlClass} ${isSplitPanelOpen && hasSplitPanelContent ? "bg-[var(--accent-background)] text-[var(--foreground)]" : "text-[var(--foreground-muted)]"} disabled:cursor-default disabled:opacity-30`}
          disabled={!hasSplitPanelContent}
          onClick={onToggleSplit}
          title={hasSplitPanelContent ? t("chat.toggle_split") : t("chat.split_unavailable")}
          type="button"
        >
          <SplitPaneIcon />{t("chat.split")}
        </button>
      </div>
    </header>
  );
}
