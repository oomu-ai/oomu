import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ShieldApprovalDialog } from "./ShieldApprovalDialog";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
afterEach(cleanup);

it("offers a clear current-chat choice for public search", () => {
  invokeMock.mockResolvedValue({ activeLocale: "en-US", availableLocales: [], translations: {} });
  const onApprove = vi.fn();
  render(
    <ShieldApprovalDialog
      isResolving={false}
      onApprove={onApprove}
      onDeny={vi.fn()}
      request={{
        approvalToken: "public-search",
        sessionId: "chat-search",
        actionType: "public_web_search",
        actionLabel: "local_search/search_web",
        actionClass: "public_web_search",
        riskTier: "network",
        reason: "Search the public web",
        requestedAtMs: Date.now(),
        preview: "",
        scopeTrustAvailable: true,
        approvalScopeKinds: ["once", "chat_session"],
      }}
    />,
    { wrapper: I18nProvider },
  );

  const scope = screen.getByRole("combobox", { name: "Approval scope" });
  expect(within(scope).getByRole("option", { name: "Just once" })).toBeVisible();
  expect(within(scope).getByRole("option", { name: "For This Chat" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "Search the public web?" })).toBeVisible();
  expect(screen.getByText("Search the public web")).toBeVisible();
  expect(screen.queryByText(/local search/i)).not.toBeInTheDocument();
  fireEvent.change(scope, { target: { value: "chat_session" } });
  fireEvent.click(screen.getByRole("button", { name: "Approve" }));
  expect(onApprove).toHaveBeenCalledWith({
    trustScope: true,
    trustScopeKind: "chat_session",
  });
});
