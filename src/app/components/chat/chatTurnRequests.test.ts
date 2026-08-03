import { describe, expect, it } from "vitest";
import { createChatTurnContext } from "@/lib/chatTurnContext";
import {
  agentPlanTurnContextRequest,
  nativeProjectTurnContextRequest,
} from "./chatTurnRequests";

describe("agent plan turn context", () => {
  it("carries the selected Project through the immutable approval boundary", () => {
    const turn = createChatTurnContext({
      turnId: "turn-project",
      generationToken: "generation-project",
      sessionId: "session-project",
      agentId: "agent-project",
      projectId: "project-selected",
      route: {
        providerId: "provider-local",
        modelId: "model-local",
        dynamicRoutingEnabled: false,
        automatedWebGroundingEnabled: false,
      },
    });

    expect(agentPlanTurnContextRequest(turn)).toEqual(
      expect.objectContaining({ projectId: "project-selected" }),
    );
    expect(nativeProjectTurnContextRequest(turn)).toEqual(
      expect.objectContaining({ project_id: "project-selected" }),
    );
  });
});
