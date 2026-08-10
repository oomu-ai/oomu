import { describe, expect, it } from "vitest";
import {
  ensurePendingAssistantMessage,
  projectChatDocumentRequest,
  projectDocumentOutputRequested,
  projectDocumentLocalExecutionRoute,
  projectDocumentNativeRequestRoute,
  projectDocumentPendingAssistantId,
  projectDocumentRequestNeedsProjectScope,
  projectDocumentRouteDecision,
} from "../projectChatDocument";

const routeDecision = {
  route: "agentic_planner" as const,
  requires_local_access: true,
  decision_source: "native_artifact_creation_filter",
  reason: "Document creation requested.",
  matched_signals: ["document"],
  status_label: "Planning…",
};

describe("Project chat document composition", () => {
  it("uses approved Project knowledge while removing the native output instruction", () => {
    const request = projectChatDocumentRequest(
      "Using only the files in this Project, prepare a two-page update. Create a results table. Produce an editable Word document and a PDF.",
      routeDecision,
      "project_11111111-1111-4111-8111-111111111111",
    );
    expect(request?.modelMessage).toContain("approved Project knowledge supplied with this turn");
    expect(request?.modelMessage).toContain("Create a results table");
    expect(request?.modelMessage).not.toContain("Produce an editable Word document");
  });

  it("does not claim Project context when the chat is not bound to a Project", () => {
    expect(projectChatDocumentRequest("Produce an editable Word document and a PDF.", routeDecision, null)).toBeNull();
  });

  it("stops an unbound multi-file Project request before model routing", () => {
    const message = [
      "Funder_Questions.pdf",
      "Cohort_Outcomes.xlsx",
      "Program_Notes.docx",
      "Produce an editable Word document and a PDF.",
    ].join("\n");
    expect(projectDocumentRequestNeedsProjectScope(message, null, 0)).toBe(true);
    expect(projectDocumentRequestNeedsProjectScope(message, "project-1", 0)).toBe(false);
    expect(projectDocumentRequestNeedsProjectScope(message, null, 1)).toBe(false);
  });

  it("recognizes the bounded Project document request before native classification", () => {
    const message = "Using only files in this Project, produce an editable Word document and a PDF.";
    expect(projectDocumentOutputRequested(message, "project_11111111-1111-4111-8111-111111111111")).toBe(true);
    expect(projectDocumentOutputRequested(message, null)).toBe(false);
    expect(projectDocumentRouteDecision(message, "project_11111111-1111-4111-8111-111111111111", "Thinking…"))
      .toMatchObject({ decision_source: "native_artifact_creation_filter" });
  });

  it("routes a PDF-only Project deliverable through native artifact creation", () => {
    const projectId = "project_11111111-1111-4111-8111-111111111111";
    const message = "Using only the files in this Project, summarize the outcomes and create a PDF document.";
    const decision = projectDocumentRouteDecision(message, projectId, "Thinking…");
    expect(decision).toMatchObject({ decision_source: "native_artifact_creation_filter" });
    const request = projectChatDocumentRequest(message, decision!, projectId);
    expect(request?.modelMessage).toContain("summarize the outcomes");
    expect(request?.modelMessage).not.toContain("create a PDF document");
  });

  it("does not treat a Project PDF source reference as an output request", () => {
    expect(projectDocumentOutputRequested("Read Funder_Questions.pdf and summarize it.", "project-1")).toBe(false);
  });
});

describe("Project chat document execution", () => {
  it("creates one reusable assistant-side progress message for the Project turn", () => {
    const createId = () => 42;
    const id = projectDocumentPendingAssistantId(
      "Prepare a Word document and a PDF.",
      "project_11111111-1111-4111-8111-111111111111",
      createId,
    );
    const messages = ensurePendingAssistantMessage([], id);
    expect(messages).toEqual([{ id: 42, role: "assistant", content: "", isPending: true }]);
    expect(ensurePendingAssistantMessage(messages, id)).toBe(messages);
  });

  it("keeps approved Project-folder evidence on the saved local route", () => {
    const request = { modelMessage: "Compose from the Project folder." };
    expect(projectDocumentLocalExecutionRoute(request, {
      localProviderId: "local-model", localModelId: "gemma-local",
      recommendedLocalProviderId: null, recommendedLocalModelId: null,
    }, { providerId: "dynamic", modelId: "dynamic" }, false)).toEqual({
      providerId: "local-model", modelId: "gemma-local",
    });
    expect(projectDocumentLocalExecutionRoute(request, {
      localProviderId: null, localModelId: null,
      recommendedLocalProviderId: null, recommendedLocalModelId: null,
    }, { providerId: "cloud", modelId: "cloud-model" }, false)).toBeNull();
  });

  it("forces Project composition local without replacing an Auto-route session binding", () => {
    expect(projectDocumentNativeRequestRoute(
      { modelMessage: "Compose from the Project folder." },
      {
        localProviderId: "local-model",
        localModelId: "gemma-local",
        recommendedLocalProviderId: null,
        recommendedLocalModelId: null,
      },
      {
        providerId: "dynamic",
        modelId: "dynamic",
        dynamicRoutingEnabled: true,
      },
      false,
      null,
      "Choose a local model.",
      [],
    )).toMatchObject({
      provider_id: "local-model",
      model_id: "gemma-local",
      dynamic_routing_override: true,
      auto_route_choice: "local",
      auto_route_cloud_confirmed: false,
      project_document_composition: true,
    });
  });
});
