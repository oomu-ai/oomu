import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApprovalDialogFrame } from "./ApprovalDialogFrame";

describe("ApprovalDialogFrame", () => {
  afterEach(cleanup);

  it("skips a disabled initial-focus control", () => {
    render(
      <ApprovalDialogFrame
        description="A clear description"
        eyebrow="Paused"
        footer={
          <>
            <button data-approval-initial-focus disabled type="button">
              Disabled cancel
            </button>
            <button type="button">Available choice</button>
          </>
        }
        onDismiss={vi.fn()}
        title="Review this choice"
      >
        <p>Permission details</p>
      </ApprovalDialogFrame>,
    );

    expect(screen.getByRole("button", { name: "Available choice" })).toHaveFocus();
  });

  it("focuses the dialog when every control is disabled", () => {
    render(
      <ApprovalDialogFrame
        description="A clear description"
        eyebrow="Paused"
        footer={
          <button data-approval-initial-focus disabled type="button">
            Disabled cancel
          </button>
        }
        onDismiss={vi.fn()}
        title="Review this choice"
      >
        <p>Permission details</p>
      </ApprovalDialogFrame>,
    );

    expect(screen.getByRole("dialog")).toHaveFocus();
  });
});
