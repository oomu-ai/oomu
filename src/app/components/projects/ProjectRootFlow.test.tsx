import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ProjectsScreen } from "./ProjectsScreen";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
afterEach(cleanup);

it("chooses one Project folder without importing it into Knowledge", async () => {
  const projectId = "project_11111111-1111-4111-8111-111111111111";
  const root = {
    sourceId: "source_44444444-4444-4444-8444-444444444444",
    projectId,
    sourceKind: "local_folder",
    canonicalPath: "/Users/example/Large Project",
    grantState: "active",
    indexingState: "ready",
    fileCount: 0,
    lastIndexedAtMs: null,
    failureCode: null,
    updatedAtMs: 4,
  };
  let sources: typeof root[] = [];
  invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    if (command === "get_locale_state") return {
      activeLocale: "en-US",
      availableLocales: [{ id: "en-US", label: "English (US)", fileName: "en-US.json", isDefault: true, verified: true }],
      translations: {},
    };
    if (command === "list_projects") return [{
      projectId, name: "Alpha", description: "", dataPolicy: "local_only", instructions: "",
      archivedAtMs: null, createdAtMs: 1, updatedAtMs: 2, sourceCount: sources.length,
      conversationCount: 0, workflowCount: 0, taskCount: 0,
    }];
    if (command === "list_project_sources") return sources;
    if (command === "get_project_memory_summary") return { memoryCount: 0, sourceSessions: [] };
    if (command === "choose_project_root") {
      expect(args).toEqual({ request: { projectId } });
      sources = [root];
      return root;
    }
    return null;
  });

  render(<ProjectsScreen onOpenChat={vi.fn()} onOpenWorkflows={vi.fn()} />, { wrapper: I18nProvider });
  await screen.findByRole("heading", { name: "Alpha" });
  expect(screen.getByText(/does not import the whole folder into Knowledge/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Choose folder" }));

  expect(await screen.findByText(root.canonicalPath)).toBeVisible();
  expect(screen.getByRole("button", { name: "Change folder" })).toBeVisible();
  expect(screen.getByText("No knowledge folders are attached.")).toBeVisible();
  expect(invokeMock.mock.calls.some(([command]) => command === "ingest_knowledge")).toBe(false);
  expect(invokeMock.mock.calls.some(([command]) => command === "attach_project_source")).toBe(false);
});
