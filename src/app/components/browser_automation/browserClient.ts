import { invoke } from "@/lib/invoke";

type SemanticNode = {
  role: string;
  name: string;
  valueClass: string;
  visible: boolean;
  enabled: boolean;
  reference: string;
};

export type BrowserAutomationState =
  | "automating"
  | "paused"
  | "takeover"
  | "return_pending"
  | "stopped"
  | "closed";

export type BrowserSnapshot = {
  documentGeneration: number;
  url: string;
  title: string;
  capturedAtMs: number;
  nodes: SemanticNode[];
  possiblePromptInjection: boolean;
  protectedInterruption: string | null;
};

export type BrowserSession = {
  sessionId: string;
  taskRunId: string;
  projectId: string;
  canonicalOrigin: string;
  destinationBinding: string;
  state: BrowserAutomationState;
  documentGeneration: number;
  currentStep: string;
  lastSnapshotAtMs: number | null;
  snapshot: BrowserSnapshot | null;
};

type BrowserAction =
  | { kind: "snapshot" | "screenshot" | "close" }
  | { kind: "click" | "download_to_quarantine"; reference: string }
  | { kind: "type"; reference: string; text: string }
  | { kind: "select"; reference: string; value: string }
  | { kind: "press_key"; key: string }
  | { kind: "scroll"; deltaY: number }
  | { kind: "wait"; milliseconds: number }
  | { kind: "navigate"; url: string }
  | { kind: "upload_approved_file"; reference: string; uploadGrantId: string };

export type BrowserActionResult = {
  state: string;
  observation: BrowserSnapshot | null;
  screenshotPath: string | null;
  message: string;
};

export const browserAutomationApi = {
  start: (
    taskRunId: string,
    projectId: string,
    projectPolicyConsent: boolean,
  ) => invoke<BrowserSession>("start_browser_automation", {
    request: { taskRunId, projectId, projectPolicyConsent },
  }),

  get: (sessionId: string, taskRunId: string) =>
    invoke<BrowserSession>("get_browser_automation_session", {
      request: { sessionId, taskRunId },
    }),

  control: (
    sessionId: string,
    taskRunId: string,
    control: "pause" | "takeover" | "return" | "stop",
  ) => invoke<BrowserSession>("control_browser_automation", {
    request: { sessionId, taskRunId, control },
  }),

  action: (
    session: BrowserSession,
    action: BrowserAction,
    step: string,
    expectedPostcondition?: string,
  ) => invoke<BrowserActionResult>("execute_browser_action", {
    request: {
      sessionId: session.sessionId,
      taskRunId: session.taskRunId,
      projectId: session.projectId,
      action,
      step,
      expectedPostcondition: expectedPostcondition || null,
    },
  }),

  chooseUpload: (sessionId: string, taskRunId: string) =>
    invoke<{
      uploadGrantId: string;
      fileName: string;
      mimeType: string;
      byteCount: number;
    } | null>("choose_browser_upload", {
      request: { sessionId, taskRunId },
    }),
};
