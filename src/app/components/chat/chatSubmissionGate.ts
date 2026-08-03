export type ChatSubmissionGate = {
  activeSessionMismatch: boolean;
  hasTurnContent: boolean;
  hasSelectedAgent: boolean;
  isSending: boolean;
  isReadingAttachments: boolean;
  queueIsExecuting: boolean;
  submissionIsPending: boolean;
};

export function chatSubmissionIsBlocked(gate: ChatSubmissionGate) {
  return gate.activeSessionMismatch ||
    !gate.hasTurnContent ||
    !gate.hasSelectedAgent ||
    gate.isSending ||
    gate.isReadingAttachments ||
    gate.queueIsExecuting ||
    gate.submissionIsPending;
}

export type LocalModelHydrationGate = {
  isRecovery: boolean;
  localModelIsHydrating: boolean;
  hasDirectLocalCommand: boolean;
  hasDirectLocalRead: boolean;
  hasDirectMailRead: boolean;
  hasDirectCalendarRead: boolean;
  hasDirectAppleRead: boolean;
  hasDirectAppleWrite: boolean;
  isSystemDiagnostics: boolean;
  likelyLocalNativeTask: boolean;
  hasAmbiguousLocalAppIntent: boolean;
};

export function shouldWaitForLocalModelHydration(gate: LocalModelHydrationGate) {
  return !gate.isRecovery &&
    gate.localModelIsHydrating &&
    !gate.hasDirectLocalCommand &&
    !gate.hasDirectLocalRead &&
    !gate.hasDirectMailRead &&
    !gate.hasDirectCalendarRead &&
    !gate.hasDirectAppleRead &&
    !gate.hasDirectAppleWrite &&
    !gate.isSystemDiagnostics &&
    !gate.likelyLocalNativeTask &&
    !gate.hasAmbiguousLocalAppIntent;
}

type ReplayAcceptedTurn = {
  sessionId: string;
};

type SubmissionSeedOptions<TResume extends ReplayAcceptedTurn> = {
  recoveryPlan?: boolean;
  expectedSessionId?: string;
  resumeAcceptedTurn?: TResume;
};

export function createChatSubmissionSeed<T, TResume extends ReplayAcceptedTurn>(
  nextMessageValue: string,
  options: SubmissionSeedOptions<TResume>,
  attachments: T[],
  activeSessionId: string,
) {
  const recoveryPlan = options.recoveryPlan === true;
  const resume = options.resumeAcceptedTurn;
  const turnFiles = recoveryPlan || resume ? [] : attachments;
  return {
    submittedMessage: nextMessageValue,
    nextMessage: nextMessageValue.trim(),
    recoveryPlan,
    resume,
    turnFiles,
    replayOrRecovery: Boolean(recoveryPlan || resume),
    hasTurnContent: Boolean(nextMessageValue.trim() || turnFiles.length),
    submitSession: recoveryPlan
      ? options.expectedSessionId?.trim() ?? ""
      : resume?.sessionId.trim() || activeSessionId,
  };
}

export function isRecoverySubmission(
  recoveryPlan: boolean,
  hasCalendarFollowup: boolean,
  hasResumedTurn: boolean,
) {
  return recoveryPlan || hasCalendarFollowup || hasResumedTurn;
}

export function recoverySessionMismatch(
  replayOrRecovery: boolean,
  submitSession: string,
  activeSessionId: string,
) {
  return replayOrRecovery && submitSession !== activeSessionId.trim();
}
