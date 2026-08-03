import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ShieldApprovalDialog, type ShieldApprovalRequest } from "./ShieldApprovalDialog";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

describe("Shield file-write approval location", () => {
  it("shows the exact canonical target beside the primary decision", () => {
    invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
    const targetPath = "/Users/example/Desktop/OOMU reports/oomu-test-diagnostic-report.md";
    const request: ShieldApprovalRequest = {
      approvalToken: "file-write-location",
      actionType: "file_write",
      actionLabel: "Write file",
      actionClass: "filesystem_write",
      targetPath,
      riskTier: "file_write",
      reason: "Create the requested report.",
      requestedAtMs: Date.now(),
      preview: "",
    };

    const { container } = render(
      <ShieldApprovalDialog
        isResolving={false}
        onApprove={vi.fn()}
        onDeny={vi.fn()}
        request={request}
      />,
      { wrapper: I18nProvider },
    );
    const location = container.querySelector<HTMLElement>(
      '[data-oomu-file-write-approval-location="true"]',
    );

    expect(location).toBeVisible();
    expect(location).toHaveAttribute("role", "group");
    expect(location).toHaveAccessibleName("Location");
    expect(within(location!).getByText("Location")).toBeVisible();
    const path = location?.querySelector<HTMLElement>("[data-oomu-file-write-target-path]");
    expect(path).toBeVisible();
    expect(path?.textContent).toBe(targetPath);
    expect(screen.getByRole("button", { name: "Allow Once" })).toBeEnabled();
  });
});
