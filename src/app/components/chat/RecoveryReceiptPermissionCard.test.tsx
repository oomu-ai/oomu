import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { I18nProvider } from "@/context/I18nContext";
import { describe, expect, it, vi } from "vitest";
import { RecoveryReceiptCard } from "./RecoveryReceiptCard";

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn(async () => ({
    activeLocale: "en-US",
    availableLocales: [],
    translations: {},
  })),
}));

const contactsReceipt = JSON.stringify({
  schema: "oomu.agent_execution_recovery.v1",
  executionId: "execution-contacts-301",
  planId: "plan-contacts-301",
  code: "contacts_permission_denied",
  boundary: "contacts_authorization",
  recoverable: true,
  recoveryAction: "resume_same_execution",
  message: "Contacts access is needed.",
  changedState: "checkpoint_saved",
  context: { capabilityId: "contacts" },
});

describe("permission receipt presentation", () => {
  it("turns a typed Contacts receipt into one saved-turn recovery path", async () => {
    const onCheck = vi.fn(async () => undefined);
    const onOpen = vi.fn(async () => undefined);
    render(
      <RecoveryReceiptCard
        content={contactsReceipt}
        onCheckMacPermissionAccess={onCheck}
        onOpenMacPermissionSettings={onOpen}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getByRole("alert", { name: "Contacts access needed" })).toHaveFocus();
    expect(screen.getByText(
      "Your request is saved. Nothing else changed. OOMU can continue from this point after access is ready.",
    ))
      .toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Open System Settings" }));
    await waitFor(() => expect(onOpen).toHaveBeenCalledWith(
      "execution-contacts-301",
      "contacts",
    ));
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    await waitFor(() => expect(onCheck).toHaveBeenCalledWith(
      "execution-contacts-301",
      "contacts",
    ));
  });
});
