import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY,
  useDecisionBriefCompletion,
} from "./firstRunWelcomeState";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

describe("decision brief completion for first run", () => {
  beforeEach(() => {
    const values = new Map<string, string>();
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      value: {
        getItem: vi.fn((key: string) => values.get(key) ?? null),
        removeItem: vi.fn((key: string) => values.delete(key)),
        setItem: vi.fn((key: string, value: string) => values.set(key, value)),
      },
    });
    invokeMock.mockReset();
  });

  it("suppresses first-run cards for a completed brief restored without a local flag", async () => {
    invokeMock.mockImplementation(async (command: string) =>
      command === "list_projects"
        ? [{ projectId: "project-complete" }]
        : { readyOnDemand: true, readyWeekly: false },
    );

    const { result } = renderHook(() => useDecisionBriefCompletion());
    await waitFor(() => expect(result.current).toBe("complete"));
    expect(window.localStorage.getItem(FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY)).toBe("1");
    expect(invokeMock).toHaveBeenCalledWith(
      "get_weekly_decision_brief_status",
      { request: { projectId: "project-complete" } },
    );
  });

  it("allows a true first run when there are no projects", async () => {
    invokeMock.mockResolvedValue([]);
    const { result } = renderHook(() => useDecisionBriefCompletion());

    await waitFor(() => expect(result.current).toBe("incomplete"));
    expect(window.localStorage.getItem(FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY)).toBeNull();
  });

  it("fails quiet when completion cannot be checked", async () => {
    invokeMock.mockRejectedValue(new Error("BACKEND CANARY"));
    const { result } = renderHook(() => useDecisionBriefCompletion());

    await waitFor(() => expect(result.current).toBe("unavailable"));
    expect(window.localStorage.getItem(FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY)).toBeNull();
  });
});
