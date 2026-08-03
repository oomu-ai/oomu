import { describe, expect, it } from "vitest";
import {
  destinationForConnectionsSection,
  destinationForTasksSection,
  resolveChatStarterDestination,
  resolveHeroDestination,
} from "./homeNavigation";

describe("home navigation contracts", () => {
  it("routes all three first-run outcomes to real destinations", () => {
    expect(resolveChatStarterDestination("weekly_brief")).toEqual({
      item: "hero",
      templateId: null,
    });
    expect(resolveChatStarterDestination("summarize_folder")).toEqual({
      item: "workflows",
      templateId: "directory-summarizer",
    });
    expect(resolveChatStarterDestination("help_with_email")).toEqual({
      item: "workflows",
      templateId: "email-responder",
    });
  });

  it("enumerates hero backend destinations with a safe fallback", () => {
    expect(resolveHeroDestination("integrations")).toBe("integrations");
    expect(resolveHeroDestination("routines")).toBe("routines");
    expect(resolveHeroDestination("artifacts")).toBe("artifacts");
    expect(resolveHeroDestination("unknown_backend_destination")).toBe("settings");
  });

  it("routes merged section controls through the legacy resolver inputs", () => {
    expect(destinationForTasksSection("now")).toBe("tasks");
    expect(destinationForTasksSection("scheduled")).toBe("routines");
    expect(destinationForTasksSection("workflows")).toBe("workflows");
    expect(destinationForConnectionsSection("work_apps")).toBe("integrations");
    expect(destinationForConnectionsSection("messaging")).toBe("channels");
  });
});
