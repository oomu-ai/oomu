import type { Dispatch, SetStateAction } from "react";
import { configToAgent, type AgentCardData, type AgentConfigRecord } from "./homeAgents";

export function importedAgentRefreshAction(
  agent: AgentCardData,
  refresh: (agent: AgentCardData) => void,
) {
  if (agent.type === "archived" || !agent.id.startsWith("imported_")) return undefined;
  return () => refresh(agent);
}

export function importedAgentRefreshTarget(
  agent: AgentCardData | null,
  defaultModelId: string,
) {
  if (!agent) return undefined;
  return {
    id: agent.id,
    name: agent.name,
    description: agent.description,
    providerId: agent.endpoint?.provider ?? "local_model",
    modelId: agent.endpoint?.modelId ?? defaultModelId,
  };
}

export function commitImportedAgent(
  config: AgentConfigRecord,
  refreshAgent: AgentCardData | null,
  setActive: Dispatch<SetStateAction<AgentCardData[]>>,
  setArchived: Dispatch<SetStateAction<AgentCardData[]>>,
  setSelected: Dispatch<SetStateAction<AgentCardData | null>>,
) {
  const agent = configToAgent(config);
  if (!refreshAgent) {
    setActive((current) => [...current, agent]);
    return;
  }
  const replace = (current: AgentCardData[]) => current.map((item) => item.id === agent.id ? agent : item);
  setActive(replace);
  setArchived(replace);
  setSelected(agent);
}
