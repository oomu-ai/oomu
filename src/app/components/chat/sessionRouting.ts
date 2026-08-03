import type { AgentPersonalityProfile } from "@/lib/agentPersonality";
import type { ReasoningLevel } from "@/lib/modelRegistry";

export const DYNAMIC_ROUTE_ID = "dynamic";
type RouteProviderId = string;

export type RouteOverride = {
  providerId: RouteProviderId;
  providerType: string;
  modelId: string;
  reasoning: ReasoningLevel;
  context: string;
};

export type ChatSessionRouteBinding = {
  providerId: string;
  modelId: string;
  dynamicRoutingOverride?: boolean | null;
  autoRouteBaseline?: RouteOverride;
};

export function routeBindingForDynamicRouting(
  dynamicRoutingEnabled: boolean,
  route: RouteOverride,
): ChatSessionRouteBinding {
  const binding = dynamicRoutingEnabled
    ? { providerId: DYNAMIC_ROUTE_ID, modelId: DYNAMIC_ROUTE_ID }
    : { providerId: route.providerId, modelId: route.modelId };
  return {
    ...binding,
    dynamicRoutingOverride: dynamicRoutingEnabled ? true : undefined,
    autoRouteBaseline: dynamicRoutingEnabled ? { ...route } : undefined,
  };
}

export function dynamicRoutingDefaultForAgent(agent?: {
  personalityProfile?: AgentPersonalityProfile;
} | null) {
  return agent?.personalityProfile?.modelBehavior?.dynamicRoutingDefault === true;
}
