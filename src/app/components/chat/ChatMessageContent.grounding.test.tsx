import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ChatMessageContent } from "./ChatMessageContent";

describe("ChatMessageContent grounding accessibility", () => {
  it("keeps verified source links inside the assistant response document", () => {
    const sourceUrl = "https://www.rust-lang.org/tools/install";
    render(
      <ChatMessageContent
        accessibilityId="oomu-assistant-response-test"
        content="Rust release details from the verified source."
        role="assistant"
        sources={[{
          url: sourceUrl,
          accessedAtUtc: "2026-07-24T10:00:00.000Z",
        }]}
      />,
      { wrapper: I18nProvider },
    );

    const response = screen.getByRole("document");
    expect(response).toHaveAttribute("id", "oomu-assistant-response-test");
    expect(within(response).getByRole("link", { name: sourceUrl })).toHaveAttribute(
      "href",
      sourceUrl,
    );
  });
});
