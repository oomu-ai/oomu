import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const chatScreenPath = "src/app/components/ChatScreen.tsx";
const chatWorkspaceChromePath = "src/app/components/chat/ChatWorkspaceChrome.tsx";
const routingIndicatorPath = "src/app/components/chat/RoutingIndicator.tsx";

describe("ChatScreen presentation source contract", () => {
  it("keeps first-run state and the agent doorway delegated to extracted components", () => {
    const source = readFileSync(chatScreenPath, "utf8");

    expect(source).toContain("<ChatEmptyState");
    expect(source).toContain("sessionsLoaded={sessionsLoaded}");
    expect(source).toContain("sessionCount={sessions.length}");
    expect(source).toContain("transcriptEmpty={messages.length === 0}");
    expect(source).toContain("<ChatWorkspaceHeader");
    expect(source).toContain("onManageAgents={onManageAgents}");
    expect(readFileSync(chatWorkspaceChromePath, "utf8")).toContain("<ChatAgentSelector");
  });

  it("keeps the route chip in natural case without letter spacing", () => {
    const source = readFileSync(routingIndicatorPath, "utf8");
    expect(source).toContain("export function RoutingIndicator(");
    expect(source).not.toMatch(/\buppercase\b|\btracking-/);
    expect(readFileSync(chatScreenPath, "utf8")).toContain("<RoutingIndicator");
  });

  it("keeps retrieval telemetry behind the developer-build fence", () => {
    const source = readFileSync(chatScreenPath, "utf8");

    expect(source).toContain('import { isDeveloperBuild } from "@/lib/buildFlags";');
    expect(source).toContain("isDeveloperBuild && headlessSearchDebug &&");
  });

  it("does not grow the ChatScreen line ratchet", () => {
    const source = readFileSync(chatScreenPath, "utf8");
    expect(source.split("\n").length - 1).toBeLessThanOrEqual(9857);
  });
});
