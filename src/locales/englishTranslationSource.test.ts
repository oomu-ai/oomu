import { readdirSync } from "node:fs";
import { describe, expect, it } from "vitest";
import enUS from "./en-US.json";

function valueAtPath(path: string) {
  return path.split(".").reduce<unknown>((value, key) => {
    if (!value || typeof value !== "object") return undefined;
    return (value as Record<string, unknown>)[key];
  }, enUS);
}

function leafValues(value: unknown): string[] {
  if (typeof value === "string") return [value];
  if (!value || typeof value !== "object" || Array.isArray(value)) return [];
  return Object.values(value).flatMap(leafValues);
}

describe("English translation source", () => {
  it("keeps user copy in the gated JSON source instead of TypeScript fragments", () => {
    const fragments = readdirSync("src/locales").filter((file) =>
      file.endsWith("Translations.ts"),
    );

    expect(fragments).toEqual([]);
  });

  it.each([
    "approvals.approve_for_workflow",
    "chat.auto_route_attention.attention_content",
    "chat.errors.connector_authority.content",
    "chat.execution.stopped",
    "chat.project_cloud_consent.body",
    "chat.project_scope.new_project_chat",
    "chat.recovery.calendar_title",
    "chat.recovery.mail_automation_permission_title",
    "projects.knowledge_help",
    "routines.history_failure_title",
    "sprint_301.auto_route_recovery.choose_model_title",
    "sprint_301.permission_recovery.denied_body",
    "sprint_301.route.details_ready",
    "tasks.effect_verification_title",
    "workflows.scope.choose_project",
    "workflows.trust.actions.deliver_configured_channel",
  ])("resolves %s from en-US.json", (path) => {
    expect(valueAtPath(path)).toEqual(expect.any(String));
    expect((valueAtPath(path) as string).trim()).not.toBe("");
  });

  it("keeps the privacy panel free of implementation jargon", () => {
    const privacyCopy = leafValues(enUS.settings.privacy).join("\n");

    expect(privacyCopy).not.toMatch(
      /grounding|keyless|after a challenge|hidden browser|persistent cookies|remote proxy services|search API keys|reviewed public sources/i,
    );
  });
});
