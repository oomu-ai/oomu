import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { PersistenceLoadNotice } from "./PersistenceLoadNotice";

describe("PersistenceLoadNotice", () => {
  it("presents one accessible retry without allowing duplicate work", () => {
    const onRetry = vi.fn();
    const view = render(
      <PersistenceLoadNotice
        message="Your chats could not be loaded. Your last confirmed chats are still here."
        onRetry={onRetry}
        retryLabel="Retry"
        retrying={false}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent("last confirmed chats");
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);

    view.rerender(
      <PersistenceLoadNotice
        message="Your chats could not be loaded. Your last confirmed chats are still here."
        onRetry={onRetry}
        retryLabel="Retry"
        retrying
      />,
    );
    expect(screen.getByRole("button", { name: "Retry" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });
});
