import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { ScreenEmptyState } from "./ScreenEmptyState";

it("presents one purpose, one source sentence, and one primary action", () => {
  const onAction = vi.fn();
  render(
    <ScreenEmptyState
      actionLabel="Start here"
      body="New work appears here."
      icon={<svg aria-label="Quiet icon" />}
      onAction={onAction}
      title="Nothing here yet"
    />,
  );

  expect(screen.getByRole("heading", { name: "Nothing here yet" })).toBeVisible();
  expect(screen.getByText("New work appears here.")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Start here" }));
  expect(onAction).toHaveBeenCalledTimes(1);
});
