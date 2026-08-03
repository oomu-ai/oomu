import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { McpConfirmationModal } from "./McpConfirmationModal";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

describe("McpConfirmationModal", () => {
  afterEach(cleanup);

  beforeEach(() => {
    invokeMock.mockResolvedValue({
      activeLocale: "en-US",
      availableLocales: [],
      translations: {},
    });
  });

  it("shows only friendly destination and location details", () => {
    const view = render(
      <McpConfirmationModal
        argumentsValue={{ destination_path: "/Reports/weekly.md", recipients: ["alex@example.test"] }}
        isOpen
        onApprove={() => undefined}
        onCancel={() => undefined}
        serverName="Google Workspace"
        toolName="Create draft"
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByText("Location")).toBeVisible();
    expect(screen.getByText("weekly.md")).toBeVisible();
    expect(screen.getByText("Destination")).toBeVisible();
    expect(screen.getByText("alex@example.test")).toBeVisible();
    expect(screen.queryByText("/Reports/weekly.md")).not.toBeInTheDocument();
    expect(view.container.querySelector("pre")).toBeNull();
  });

  it("never renders tokens, code, JSON, markup, full paths, or URL details", () => {
    render(
      <McpConfirmationModal
        argumentsValue={{
          destination: "https://api.example.test/private?token=hidden",
          path: "/Users/example/Reports/board.pdf",
          purpose: "Use /Users/example/private with https://private.example.test",
          reason: "curl https://private.example.test --token hidden",
          capabilityReason: "<script>private()</script>",
          token: "raw-secret-token",
          script: "rm -rf /private/data",
          payload: { private: true },
        }}
        isOpen
        onApprove={() => undefined}
        onCancel={() => undefined}
        serverName="Local tools"
        toolName="Prepare report"
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByText("api.example.test")).toBeVisible();
    expect(screen.getByText("board.pdf")).toBeVisible();
    const dialogText = screen.getByRole("dialog").textContent ?? "";
    expect(dialogText).not.toContain("raw-secret-token");
    expect(dialogText).not.toContain("curl");
    expect(dialogText).not.toContain("rm -rf");
    expect(dialogText).not.toContain("/Users/example");
    expect(dialogText).not.toContain("?token=");
    expect(dialogText).not.toContain("payload");
    expect(dialogText).not.toContain("private: true");
  });

  it("uses a friendly fallback when no safe meaning is available", () => {
    render(
      <McpConfirmationModal
        argumentsValue={{ token: "raw-secret-token", command: "rm -rf /" }}
        isOpen
        onApprove={() => undefined}
        onCancel={() => undefined}
        serverName="Local tools"
        toolName="Protected action"
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByText("No additional information is needed.")).toBeVisible();
    expect(screen.getByRole("dialog")).not.toHaveTextContent("raw-secret-token");
    expect(screen.getByRole("dialog")).not.toHaveTextContent("rm -rf");
  });

  it("explains when an approval is safely reusable for the saved workflow", () => {
    render(
      <McpConfirmationModal
        approveLabel="Approve for this workflow"
        argumentsValue={{ destination: "eia.gov" }}
        isOpen
        onApprove={() => undefined}
        onCancel={() => undefined}
        scopeNotice="Approve this saved workflow once. OOMU asks again if the workflow, tool, or destination changes."
        serverName="OOMU"
        toolName="Fetch official page"
      />,
      { wrapper: I18nProvider },
    );

    expect(
      screen.getByRole("button", { name: "Approve for this workflow" }),
    ).toBeEnabled();
    expect(screen.getByRole("dialog")).toHaveTextContent(
      "Approve this saved workflow once. OOMU asks again if the workflow, tool, or destination changes.",
    );
  });

  it("explains a failed verification without implying the permission prompt failed", () => {
    render(
      <McpConfirmationModal
        argumentsValue={{}}
        canApprove={false}
        isOpen
        onApprove={() => undefined}
        onCancel={() => undefined}
        serverName="This Mac"
        toolName="Protected action"
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "OOMU couldn’t verify this action, so it can’t be approved.",
    );
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(screen.getByRole("dialog")).not.toHaveTextContent(
      "couldn’t ask for permission",
    );
  });

  it("closes on Escape and returns focus to the control that opened it", async () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      return <>
        <button onClick={() => setOpen(true)} type="button">Open approval</button>
        <McpConfirmationModal argumentsValue={{}} isOpen={open} onApprove={() => setOpen(false)} onCancel={() => setOpen(false)} serverName="Local tools" toolName="Read folder" />
      </>;
    }
    render(<Harness />, { wrapper: I18nProvider });
    const opener = screen.getByRole("button", { name: "Open approval" });
    opener.focus();
    fireEvent.click(opener);
    expect(screen.getByRole("button", { name: "Deny" })).toHaveFocus();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() => expect(opener).toHaveFocus());
  });

  it("locks both decisions while an approval is being resolved", () => {
    const onApprove = vi.fn();
    const onCancel = vi.fn();
    render(
      <McpConfirmationModal
        argumentsValue={{ path: "/Reports" }}
        isOpen
        isResolving
        onApprove={onApprove}
        onCancel={onCancel}
        serverName="Local tools"
        toolName="Read folder"
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("button", { name: "Deny" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Approve" })).toHaveAttribute("aria-busy", "true");
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onCancel).not.toHaveBeenCalled();
    expect(onApprove).not.toHaveBeenCalled();
  });
});
