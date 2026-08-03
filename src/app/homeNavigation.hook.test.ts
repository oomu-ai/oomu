import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useHomeWorkspaceNavigation } from "./homeNavigation";

const invokeMock = vi.hoisted(() => vi.fn());
let folderResponse: unknown;
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
vi.mock("@/context/I18nContext", () => ({
  useI18n: () => ({
    t: (key: string) =>
      ({
        "workflows.templates.directory-summarizer.picker_title":
          "Choose a folder to summarize",
        "workflows.templates.directory-summarizer.truncation_file_notice":
          "Selection incomplete notice",
      })[key] ?? key,
  }),
}));

describe("useHomeWorkspaceNavigation", () => {
  beforeEach(() => {
    folderResponse = null;
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) =>
      command === "list_projects" ? [] : folderResponse,
    );
  });

  it("stages a chosen folder before opening its real workflow", async () => {
    folderResponse = {
      fileCount: 3,
      folderName: "Case files",
      folderPath: "workspace/selections/selection-case-files",
      totalBytes: 1200,
      truncated: false,
    };
    const setActiveItem = vi.fn();
    const { result } = renderHook(() =>
      useHomeWorkspaceNavigation(setActiveItem),
    );

    await act(async () => {
      expect(
        await result.current.handleChatStarterAction("summarize_folder"),
      ).toBe(true);
    });

    expect(invokeMock).toHaveBeenCalledWith("choose_workflow_source_folder", {
      selectionId: expect.stringMatching(/^selection-[a-z0-9-]+$/i),
      title: "Choose a folder to summarize",
      truncationNotice: "Selection incomplete notice",
    });
    expect(setActiveItem).toHaveBeenCalledWith("workflows");
    expect(result.current.requestedWorkflowTemplateId).toBe("directory-summarizer");
    expect(result.current.requestedWorkflowSourceFolder).toMatchObject({
      folderName: "Case files",
      truncated: false,
    });
  });

  it("stays on the welcome when the folder picker is cancelled", async () => {
    folderResponse = null;
    const setActiveItem = vi.fn();
    const { result } = renderHook(() =>
      useHomeWorkspaceNavigation(setActiveItem),
    );

    await act(async () => {
      expect(
        await result.current.handleChatStarterAction("summarize_folder"),
      ).toBe(false);
    });

    expect(setActiveItem).not.toHaveBeenCalled();
    expect(result.current.requestedWorkflowTemplateId).toBeNull();
  });
});
