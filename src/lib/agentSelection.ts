export const ACTIVE_AGENT_STORAGE_KEY = "oomu.chat.activeAgentId";
export const PENDING_SIDEBAR_AGENT_STORAGE_KEY = "oomu.chat.pendingSidebarAgentId";
export const SIDEBAR_AGENT_SELECT_EVENT = "oomu:sidebar-agent-select";
export const ACTIVE_AGENT_CHANGED_EVENT = "oomu:active-agent-changed";

export type AgentSelectionEventDetail = {
  agentId: string;
};
