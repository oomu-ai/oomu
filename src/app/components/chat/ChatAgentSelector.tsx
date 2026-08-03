"use client";

import { useI18n } from "@/context/I18nContext";

type ChatAgentOption = {
  id: string;
  name: string;
};

function AgentsDoorwayIcon() {
  return (
    <svg
      aria-hidden="true"
      className="h-3.5 w-3.5 shrink-0"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.8"
      viewBox="0 0 24 24"
    >
      <circle cx="9" cy="8" r="3" />
      <path d="M3.5 19c.5-3.3 2.3-5 5.5-5s5 1.7 5.5 5M16 7h5M18.5 4.5v5" />
    </svg>
  );
}

export function ChatAgentSelector({
  activeAgentId,
  agents,
  controlClassName,
  onAgentChange,
  onManageAgents,
}: {
  activeAgentId: string;
  agents: readonly ChatAgentOption[];
  controlClassName: string;
  onAgentChange: (agentId: string) => void | Promise<void>;
  onManageAgents?: () => void;
}) {
  const { t } = useI18n();

  return (
    <div className="flex items-center gap-2">
      <label className={controlClassName}>
        <span>{t("chat.agent")}</span>
        <select
          className="min-w-0 cursor-pointer appearance-none bg-transparent text-sm font-semibold text-[var(--foreground)] outline-none"
          onChange={(event) => void onAgentChange(event.target.value)}
          value={activeAgentId}
        >
          {agents.map((agent) => (
            <option key={agent.id} value={agent.id}>
              {agent.name}
            </option>
          ))}
        </select>
      </label>
      {onManageAgents ? (
        <button className={controlClassName} onClick={onManageAgents} type="button">
          <AgentsDoorwayIcon />
          {t("chat.manage_agents")}
        </button>
      ) : null}
    </div>
  );
}
