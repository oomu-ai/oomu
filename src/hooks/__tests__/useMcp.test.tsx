import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  getToolResultErrorMessage,
  McpProvider,
  useOptionalMcp,
} from "../useMcp";
import { ApprovalProvider } from "@/context/ApprovalContext";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauriRuntime: true,
}));

vi.mock("@/components/mcp/McpConfirmationModal", () => ({
  McpConfirmationModal: () => null,
}));

function ServerStatusProbe() {
  const mcp = useOptionalMcp();
  const state = mcp?.getServerState("macos_applescript");
  return (
    <div>
      <span data-testid="mail-server-status">{state?.status ?? "missing"}</span>
      <span data-testid="mail-server-tools">
        {state?.tools.map((tool) => tool.name).join(",") ?? ""}
      </span>
    </div>
  );
}

function ToolExecutionProbe({
  onError,
  requestApproval,
}: {
  onError?: (error: unknown) => void;
  requestApproval: () => Promise<boolean>;
}) {
  const mcp = useOptionalMcp();
  return (
    <button
      onClick={() => void mcp
        ?.executeTool("remote", "summarize", {}, { requestApproval })
        .catch((error) => onError?.(error))}
      type="button"
    >
      Run
    </button>
  );
}

afterEach(cleanup);

describe("McpProvider catalog hydration and error normalization", () => {
  it("redacts and bounds attacker-controlled nested MCP error content", () => {
    const message = getToolResultErrorMessage("remote_tool", {
      content: [
        {
          type: "text",
          text: "Bearer bearer-secret-canary failed at https://user:pass@example.test/private?token=query-secret-canary",
        },
        {
          nested: {
            api_key: "nested-secret-canary",
            url: "https://example.test/path?access_token=nested-url-canary",
          },
        },
        { type: "text", text: "x".repeat(32 * 1024) },
      ],
      isError: true,
    });

    expect(message.length).toBeLessThanOrEqual(4096);
    for (const canary of [
      "bearer-secret-canary",
      "user:pass",
      "query-secret-canary",
      "nested-secret-canary",
      "nested-url-canary",
    ]) {
      expect(message).not.toContain(canary);
    }
    expect(message).toContain("[redacted]");
    expect(message).toContain("...[truncated]");
  });

  it("connects built-in servers and hydrates their live tool schemas", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "mcp_builtin_server_configs") {
        return [
          {
            name: "macos_applescript",
            command: "python3",
            args: ["mcp_applescript.py"],
            env: {},
            transport: { type: "stdio" },
          },
        ];
      }
      if (command === "mcp_connect_builtin_server") {
        return {
          name: "macos_applescript",
          status: "connected",
          tools: [{
            name: "read_system_calendar",
            description: "Read Calendar",
            inputSchema: { type: "object" },
          }],
        };
      }
      throw new Error(`Unexpected invoke: ${command}`);
    });

    render(
      <ApprovalProvider>
        <McpProvider>
          <ServerStatusProbe />
        </McpProvider>
      </ApprovalProvider>,
    );

    await waitFor(() =>
      expect(screen.getByTestId("mail-server-status")).toHaveTextContent("connected"),
    );
    expect(screen.getByTestId("mail-server-tools")).toHaveTextContent(
      "read_system_calendar",
    );
    expect(invokeMock).toHaveBeenCalledWith("mcp_builtin_server_configs");
    expect(invokeMock).toHaveBeenCalledWith("mcp_connect_builtin_server", {
      serverName: "macos_applescript",
    });
    expect(invokeMock.mock.calls.map(([command]) => command)).not.toContain(
      "mcp_connect_server",
    );
  });
});

describe("McpProvider Shield approval continuity", () => {
  it("does not ask twice after the native Shield sheet approved the exact remote call", async () => {
    const requestApproval = vi.fn().mockResolvedValue(true);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "mcp_builtin_server_configs") return [];
      if (command === "mcp_prepare_tool_approval") {
        return {
          approvalToken: "mcp-once-token",
          serverName: "remote",
          toolName: "summarize",
          arguments: {},
          message: "approved",
          expiresAtMs: Date.now() + 60_000,
          nativeShieldApproved: true,
        };
      }
      if (command === "mcp_execute_tool") {
        return { content: [{ type: "text", text: "done" }], isError: false };
      }
      throw new Error(`Unexpected invoke: ${command}`);
    });

    render(
      <ApprovalProvider>
        <McpProvider>
          <ToolExecutionProbe requestApproval={requestApproval} />
        </McpProvider>
      </ApprovalProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Run" }));

    await waitFor(() =>
      expect(invokeMock.mock.calls.map(([command]) => command)).toContain(
        "mcp_execute_tool",
      ),
    );
    expect(requestApproval).not.toHaveBeenCalled();
  });

  it("preserves a stable denial code after a local approval is declined", async () => {
    const onError = vi.fn();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "mcp_builtin_server_configs") return [];
      if (command === "mcp_prepare_tool_approval") {
        return {
          approvalToken: "mcp-denied-token",
          serverName: "remote",
          toolName: "summarize",
          arguments: {},
          message: "Review access",
          expiresAtMs: Date.now() + 60_000,
        };
      }
      if (command === "mcp_reject_tool_approval") return null;
      throw new Error(`Unexpected invoke: ${command}`);
    });

    render(
      <ApprovalProvider>
        <McpProvider>
          <ToolExecutionProbe
            onError={onError}
            requestApproval={vi.fn().mockResolvedValue(false)}
          />
        </McpProvider>
      </ApprovalProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Run" }));

    await waitFor(() => expect(onError).toHaveBeenCalledTimes(1));
    expect(onError.mock.calls[0][0]).toMatchObject({
      code: "shield_approval_denied",
    });
  });
});
