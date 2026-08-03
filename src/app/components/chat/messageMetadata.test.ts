import { describe, expect, it } from "vitest";
import {
  isInternalUiOnlyCheckpoint,
  localizedAssistantTerminalContent,
  localizedUiCheckpointContent,
  normalizeChatMessageMetadata,
  permissionRestoredPresentation,
} from "./messageMetadata";

describe("chat message metadata", () => {
  it("preserves the typed UI-only freshness checkpoint for localized restoration", () => {
    expect(normalizeChatMessageMetadata(JSON.stringify({
      checkpointForTurnId: "turn-1",
      checkpointKind: "web_grounding_unavailable",
      localizationKey: "chat.search_errors.ambient_unavailable",
      uiOnlyCheckpoint: true,
    }))).toEqual({
      checkpointForTurnId: "turn-1",
      checkpointKind: "web_grounding_unavailable",
      localizationKey: "chat.search_errors.ambient_unavailable",
      uiOnlyCheckpoint: true,
    });
  });

  it("restores the immutable turn owner needed for native research continuation", () => {
    expect(normalizeChatMessageMetadata({
      turn_id: "turn-browser-research",
      root_turn_id: "turn-browser-research",
      generation_token: "generation-browser-research",
    })).toMatchObject({
      turnId: "turn-browser-research",
      rootTurnId: "turn-browser-research",
      generationToken: "generation-browser-research",
    });
  });

  it("announces restored access only when a real native receipt is bound", () => {
    const metadata = normalizeChatMessageMetadata({
      checkpointKind: "permission_recovery_restored",
      localizationKey: "sprint_301.permission_recovery.restored",
      permissionRestoredForTurnId: "turn-calendar-301",
      nativeReceiptId: "apple-operation-301",
      capabilityId: "calendar",
      uiOnlyCheckpoint: true,
    });
    const translate = (key: string, variables?: Record<string, string | number>) =>
      key.endsWith(".name") ? "Calendar" : `${key}:${variables?.capability}`;
    expect(localizedUiCheckpointContent(metadata, translate)).toBe(
      "sprint_301.permission_recovery.restored:Calendar",
    );
    expect(permissionRestoredPresentation(metadata)?.attributes).toMatchObject({
      "aria-live": "polite",
      "data-native-receipt-id": "apple-operation-301",
      "data-permission-recovery-state": "restored",
      role: "status",
    });
    expect(permissionRestoredPresentation({
      checkpointKind: "permission_recovery_restored",
    })).toBeNull();
  });

  it("keeps sovereign search evidence durable but out of the UI transcript", () => {
    for (const checkpointKind of [
      "sovereign_search_progress",
      "verified_sovereign_search",
    ]) {
      expect(isInternalUiOnlyCheckpoint({ checkpointKind, uiOnlyCheckpoint: true })).toBe(true);
    }
    expect(isInternalUiOnlyCheckpoint({
      checkpointKind: "sovereign_search_progress",
      uiOnlyCheckpoint: false,
    })).toBe(false);
    expect(isInternalUiOnlyCheckpoint({
      checkpointKind: "web_grounding_unavailable",
      uiOnlyCheckpoint: true,
    })).toBe(false);
  });

  it("preserves only exact HTTPS grounding provenance", () => {
    expect(normalizeChatMessageMetadata({
      publicGroundingProvenance: [
        {
          url: "https://www.eia.gov/petroleum/gasdiesel/",
          accessedAtUtc: "2026-07-23T14:12:13.456Z",
        },
        {
          url: "file:///private/etc/passwd",
          accessedAtUtc: "2026-07-23T14:12:13.456Z",
        },
        {
          url: "https://example.test/vague",
          accessedAtUtc: "Current Turn",
        },
      ],
    })).toEqual({
      publicGroundingProvenance: [{
        url: "https://www.eia.gov/petroleum/gasdiesel/",
        accessedAtUtc: "2026-07-23T14:12:13.456Z",
      }],
    });
  });

  it("restores a typed context-condensation disclosure", () => {
    expect(normalizeChatMessageMetadata({
      context_condensed: true,
      context_budget_tokens: 8192,
      context_sources_preserved: true,
    })).toEqual({
      contextCondensed: true,
      contextBudgetTokens: 8192,
      contextSourcesPreserved: true,
    });
  });

  it("localizes only the terminal sentinel and preserves an exact evidence deficit", () => {
    const translate = (key: string) => `localized:${key}`;
    expect(localizedAssistantTerminalContent(
      "search_incomplete",
      { finishReason: "search_incomplete" },
      translate,
    )).toBe("localized:chat.search_errors.search_incomplete");
    expect(localizedAssistantTerminalContent(
      "The official Node.js source did not provide a release date.",
      { finishReason: "search_incomplete" },
      translate,
    )).toBe("The official Node.js source did not provide a release date.");
  });
});
