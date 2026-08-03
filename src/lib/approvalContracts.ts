type ApprovalScopeKind =
  | "once"
  | "app_session"
  | "task"
  | "project_path"
  | "persistent"
  | (string & {});

export type ShieldApprovalRequest = {
  approvalToken: string;
  sessionId?: string | null;
  turnId?: string | null;
  generationToken?: string | null;
  actionType: string;
  actionLabel: string;
  targetPath?: string | null;
  principal?: string | null;
  riskTier: string;
  reason: string;
  estimatedTokenCosts?: number | null;
  requestedAtMs: number;
  preview: string;
  semanticSummary?: string | null;
  semanticDetail?: string | null;
  approvalTier?: string | null;
  approvalMode?: string | null;
  diffPreview?: string | null;
  scopeTrustAvailable?: boolean;
  scopeTrustPrefix?: string | null;
  scopeTrustDurationMs?: number | null;
  projectId?: string | null;
  taskRunId?: string | null;
  actionClass?: string | null;
  argumentClass?: string | null;
  canonicalResource?: string | null;
  mandatoryReconfirm?: boolean;
  approvalScopeKinds?: ApprovalScopeKind[];
};

export type ShieldApprovalStatus = {
  displayId: string;
  sessionId?: string | null;
  actionLabel: string;
  semanticSummary: string;
  requestedAtMs: number;
  pending: boolean;
};

export type ShieldApprovalDecision = "approve" | "deny";

export type ShieldApprovalDecisionOptions = {
  trustScope?: boolean;
  trustScopeKind?: ApprovalScopeKind;
};

export type ApprovalResult = {
  decision: ShieldApprovalDecision;
  scopeKind: ApprovalScopeKind;
};
