import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { upsertByNumericId, useSessionScopedState } from "./sessionScopedState";

function AttachmentSessionHarness() {
  const [sessionId, setSessionId] = useState("chat-one");
  const [attachments, setAttachments] = useSessionScopedState<string[]>(sessionId, []);
  return (
    <>
      <output>{sessionId}:{attachments.join(",")}</output>
      <button onClick={() => setAttachments((current) => [...current, "plan.md"])}>Attach</button>
      <button onClick={() => setAttachments([])}>Remove</button>
      <button onClick={() => setSessionId("chat-one")}>Chat one</button>
      <button onClick={() => setSessionId("chat-two")}>Chat two</button>
    </>
  );
}

describe("attachment session state", () => {
  it("keeps attachment chips with their chat until the user removes them", () => {
    render(<AttachmentSessionHarness />);
    fireEvent.click(screen.getByRole("button", { name: "Attach" }));
    expect(screen.getByText("chat-one:plan.md")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Chat two" }));
    expect(screen.getByText("chat-two:")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Chat one" }));
    expect(screen.getByText("chat-one:plan.md")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(screen.getByText("chat-one:")).toBeVisible();
  });
});

describe("session transcript identity", () => {
  it("replaces a hydrated accepted message instead of rendering its durable id twice", () => {
    const current = [{ id: 6, content: "hydrated" }, { id: 7, content: "next" }];
    expect(upsertByNumericId(current, { id: 6, content: "accepted" })).toEqual([
      { id: 6, content: "accepted" },
      { id: 7, content: "next" },
    ]);
    expect(upsertByNumericId(current, { id: 8, content: "new" })).toHaveLength(3);
  });
});
