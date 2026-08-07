export type ChatIntentRouteDecision = {
  route: "conversational_stream" | "agentic_planner";
  requires_local_access: boolean;
  decision_source: string;
  reason: string;
  matched_signals: string[];
  status_label: string;
};

export type WorkspaceDataResource = "mail" | "calendar" | "reminders" | "notes" | "contacts" | "photos" | "music" | "apple_app_ui";

export function shouldDelegateToTaskFlow(message: string, routeDecision: ChatIntentRouteDecision) {
  const normalized = message.trim();
  if (!normalized) return false;
  return /\btask\s*flow\b/i.test(normalized) || /\btaskflow\b/i.test(normalized) || /\b(run|execute|compile|delegate)\b.{0,32}\bworkflow\b/i.test(normalized) || (routeDecision.route === "agentic_planner" && /\bdelegate\b.{0,32}\b(multi[-\s]?step|workflow|local task)\b/i.test(normalized));
}

export function resolveTurnProjectId(screenProjectId: string | null | undefined, sessionProjectId: string | null | undefined) {
  return sessionProjectId || screenProjectId || null;
}

export function unlessRecovery<T>(isRecovery: boolean, create: () => T): T | null {
  return isRecovery ? null : create();
}
