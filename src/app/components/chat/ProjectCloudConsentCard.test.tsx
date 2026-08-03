import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ProjectCloudConsentCard } from "./ProjectCloudConsentCard";

const copy: Record<string, string> = {
  "chat.project_cloud_consent.title": "Use the cloud for this Project?",
  "chat.project_cloud_consent.body":
    "OOMU is ready to send this message and its Project context to {destination}. Nothing has left this Mac.",
  "chat.project_cloud_consent.disclosure":
    "This choice does not approve file changes, messages, or deletions.",
  "chat.project_cloud_consent.approve_once": "Allow this message",
  "chat.project_cloud_consent.always": "Always allow for this Project",
  "chat.project_cloud_consent.cancel": "Cancel",
};

function t(key: string, values?: Record<string, string | number>) {
  return Object.entries(values ?? {}).reduce(
    (value, [name, replacement]) =>
      value.replaceAll(`{${name}}`, String(replacement)),
    copy[key] ?? key,
  );
}

describe("ProjectCloudConsentCard", () => {
  afterEach(cleanup);

  it("shows the exact destination and keeps approval choices explicit", () => {
    const onChoice = vi.fn();
    render(
      <ProjectCloudConsentCard
        attention={{ sessionId: "session-1", destination: "Gemini 3.5 Flash" }}
        onChoice={onChoice}
        t={t}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent("Gemini 3.5 Flash");
    expect(screen.getByRole("alert")).toHaveTextContent("Nothing has left this Mac");
    fireEvent.click(screen.getByRole("button", { name: "Allow this message" }));
    expect(onChoice).toHaveBeenCalledWith("approve_once");
  });

  it("offers persistent Project authority and cancellation separately", () => {
    const onChoice = vi.fn();
    render(
      <ProjectCloudConsentCard
        attention={{ sessionId: "session-1", destination: "Configured cloud model" }}
        onChoice={onChoice}
        t={t}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Always allow for this Project" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onChoice.mock.calls).toEqual([["always"], ["cancel"]]);
  });
});
