import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PrivateEgressConsentCard } from "./PrivateEgressConsentCard";

const copy: Record<string, string> = {
  "chat.private_egress_consent.title": "Send these files to {destination}?",
  "chat.private_egress_consent.body":
    "OOMU needs to send {sources} to {destination} to answer this message. Nothing has left this Mac.",
  "chat.private_egress_consent.disclosure":
    "This permission applies only to this reply. It does not approve file changes, messages, Calendar events, or deletions.",
  "chat.private_egress_consent.send_once": "Send once",
  "chat.private_egress_consent.keep_private": "Keep on this Mac",
};

function t(key: string, values?: Record<string, string | number>) {
  return Object.entries(values ?? {}).reduce(
    (value, [name, replacement]) =>
      value.replaceAll(`{${name}}`, String(replacement)),
    copy[key] ?? key,
  );
}

describe("PrivateEgressConsentCard", () => {
  afterEach(cleanup);

  it("names the files and exact destination before a one-reply approval", () => {
    const onChoice = vi.fn();
    render(
      <PrivateEgressConsentCard
        attention={{
          sessionId: "session-1",
          challengeId: "challenge-1",
          destination: "Gemini 3.6 Flash",
          sourceNames: ["plan.md", "notes.pdf"],
        }}
        onChoice={onChoice}
        t={t}
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("plan.md, notes.pdf");
    expect(alert).toHaveTextContent("Gemini 3.6 Flash");
    expect(alert).toHaveTextContent("Nothing has left this Mac");
    fireEvent.click(screen.getByRole("button", { name: "Send once" }));
    expect(onChoice).toHaveBeenCalledWith("send_once");
  });

  it("lets the user keep the sources on the Mac without implying failure", () => {
    const onChoice = vi.fn();
    render(
      <PrivateEgressConsentCard
        attention={{
          sessionId: "session-1",
          challengeId: "challenge-1",
          destination: "Gemini",
          sourceNames: ["private.md"],
        }}
        onChoice={onChoice}
        t={t}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Keep on this Mac" }));
    expect(onChoice).toHaveBeenCalledWith("keep_private");
  });
});
