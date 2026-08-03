import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { WorkflowRunFeedback } from "../WorkflowRunFeedback";

describe("WorkflowRunFeedback", () => {
  afterEach(() => cleanup());

  it("shows a complete error in an opaque inline surface", () => {
    const message =
      "OOMU couldn't reach the Apple app this workflow needs. Try again.";
    render(
      <WorkflowRunFeedback
        feedback={{ message, tone: "error" }}
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent(message);
    expect(alert).toHaveClass("bg-[var(--background)]");
    expect(alert).not.toHaveClass("fixed", "truncate", "rounded-full");
  });
});
