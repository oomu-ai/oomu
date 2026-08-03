import { invoke } from "@/lib/invoke";

type ChildResult = {
  findings: Array<{ statement: string; sourceRefs: string[]; confidence: string }>;
  sources: Array<{ sourceRef: string; sourceKind: string; digest: string; observed: boolean }>;
  uncertainties: string[];
  limitations: string[];
  complete: boolean;
  actualModelRoute: string;
  elapsedMs: number;
  inputTokensEstimate: number;
  outputTokensEstimate: number;
};

type ChildRun = {
  childRunId: string;
  goal: string;
  sourceScope: string[];
  allowedReadTools: string[];
  modelRoute: string;
  budget: { maxInputTokens: number; maxOutputTokens: number; maxToolCalls: number; timeoutMs: number; maxResponseBytes: number };
  state: "planned" | "running" | "completed" | "failed" | "cancelled" | "incomplete";
  progressSummary: string;
  result: ChildResult | null;
  errorCode: string | null;
  attempt: number;
};

export type DelegationPlan = {
  planId: string;
  projectId: string;
  taskRunId: string;
  parentModelRoute: string;
  state: string;
  synthesis: { findings: ChildResult["findings"]; uncertainties: string[]; incompleteChildRunIds: string[]; readyForParentSynthesis: boolean } | null;
  children: ChildRun[];
};

export type WorkSuggestion = { suggestionId: string; childRunId: string; kind: string; summary: string; state: "awaiting_review" | "accepted" | "rejected" | "conflict"; rejectionReason: string | null };

export const delegationApi = {
  list: (taskRunId: string) => invoke<DelegationPlan[]>("list_delegation_plans", { request: { taskRunId } }),
  execute: (planId: string) => invoke<DelegationPlan>("execute_delegation_plan", { request: { planId } }),
  cancelPlan: (planId: string) => invoke<DelegationPlan>("cancel_delegation_plan", { request: { planId } }),
  cancelChild: (planId: string, childRunId: string) => invoke<DelegationPlan>("cancel_delegation_child", { request: { planId, childRunId } }),
  retryChild: (planId: string, childRunId: string) => invoke<DelegationPlan>("retry_delegation_child", { request: { planId, childRunId } }),
  pausePlan: (planId: string) => invoke<DelegationPlan>("pause_delegation_plan", { request: { planId } }),
  resumePlan: (planId: string) => invoke<DelegationPlan>("resume_delegation_plan", { request: { planId } }),
  suggestions: (planId: string) => invoke<WorkSuggestion[]>("list_work_suggestions", { request: { planId } }),
  reviewSuggestion: (planId: string, suggestionId: string, accept: boolean, rejectionReason?: string) => invoke<WorkSuggestion[]>("review_work_suggestion", { request: { planId, suggestionId, accept, rejectionReason } }),
  createDecisionBrief: (planId: string) => invoke("create_decision_brief_from_delegation", { request: { delegationPlanId: planId, title: "Weekly Decision Brief", subtitle: "Evidence-bound research and decisions" } }),
};
