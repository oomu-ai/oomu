"use client";

import { invoke } from "@/lib/invoke";
import { useCallback, type Dispatch, type SetStateAction } from "react";
import {
  activeBadgesForBoundMods,
  type AgentCardData,
  type AgentModBadge,
  type InstalledModRecord,
} from "../homeAgents";

export function useAgentModBadgeRefresh(
  setAgentModBadges: Dispatch<SetStateAction<Record<string, AgentModBadge[]>>>,
  setAgentStateError: Dispatch<SetStateAction<string>>,
) {
  return useCallback(async (agents: AgentCardData[]) => {
    if (agents.length === 0) {
      setAgentModBadges({});
      return;
    }

    try {
      const installedMods = await invoke<InstalledModRecord[]>("list_installed_mods");
      const entries = await Promise.all(
        agents.map(async (agent) => {
          const boundModIds = await invoke<string[]>("get_agent_mods", {
            agentId: agent.id,
          });
          return [agent.id, activeBadgesForBoundMods(installedMods, boundModIds)] as const;
        }),
      );
      setAgentModBadges(Object.fromEntries(entries));
    } catch (error) {
      console.error("Failed to load agent mod bindings:", error);
      setAgentStateError("persistence_errors.agents_unavailable");
    }
  }, [setAgentModBadges, setAgentStateError]);
}
