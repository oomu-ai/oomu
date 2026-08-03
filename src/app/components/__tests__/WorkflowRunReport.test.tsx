import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import type { WorkflowIr } from "../workflowIr";
import { RunReport } from "../WorkflowRunReport";

const invokeMock = vi.hoisted(() => vi.fn(() => Promise.reject(new Error("offline"))));

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauriRuntime: false,
}));

const nodes = [
  {
    kind: "mcp_tool",
    id: "read-mail",
    label: "Read unread mail",
    serverName: "mail",
    toolName: "read_messages",
    arguments: {},
    systemTimeoutMs: 10_000,
  },
] satisfies WorkflowIr["nodes"];

describe("RunReport", () => {
  afterEach(() => {
    cleanup();
    invokeMock.mockClear();
  });

  it("renders an empty collection as successful and explains why work stopped", () => {
    render(
      <I18nProvider>
        <RunReport
          completion={{ kind: "empty_collection" }}
          durationMs={18}
          executionOrder={[]}
          nodePayloads={{
            "read-mail": { status: "Completed", output: [] },
          }}
          nodes={nodes}
          status="Completed"
        />
      </I18nProvider>,
    );

    expect(screen.getByText("Completed · Nothing found")).toHaveClass(
      "text-[var(--success)]",
    );
    expect(
      screen.getByText("Nothing matched, so no later steps ran."),
    ).toBeInTheDocument();
    expect(screen.getByText("Read unread mail")).toBeInTheDocument();
    expect(screen.getByText("Done")).toHaveClass("text-[var(--success)]");
    expect(
      screen.queryByText("This run didn't record any steps."),
    ).not.toBeInTheDocument();
  });

  it("keeps ordinary completed runs distinct from empty results", () => {
    render(
      <I18nProvider>
        <RunReport
          durationMs={null}
          executionOrder={[]}
          nodePayloads={{}}
          nodes={nodes}
          status="Completed"
        />
      </I18nProvider>,
    );

    expect(screen.getByText("Completed")).toBeInTheDocument();
    expect(screen.queryByText("Completed · Nothing found")).not.toBeInTheDocument();
    expect(
      screen.queryByText("Nothing matched, so no later steps ran."),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText("This run didn't record any steps."),
    ).toBeInTheDocument();
  });

  it("shows checkpointed steps when a later workflow step fails", () => {
    render(
      <I18nProvider>
        <RunReport
          durationMs={55_100}
          executionOrder={[]}
          nodePayloads={{
            "read-mail": {
              status: "Failed",
              error: { message: "The evidence report omitted M1." },
            },
          }}
          nodes={nodes}
          status="Failed"
        />
      </I18nProvider>,
    );

    expect(screen.getAllByText("Failed")).toHaveLength(2);
    expect(screen.getByText("Read unread mail")).toBeInTheDocument();
    expect(
      screen.getByText("The evidence report omitted M1."),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("This run didn't record any steps."),
    ).not.toBeInTheDocument();
  });
});
