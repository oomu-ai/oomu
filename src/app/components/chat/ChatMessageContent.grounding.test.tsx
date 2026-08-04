import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ChatMessageContent } from "./ChatMessageContent";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

describe("ChatMessageContent grounding accessibility", () => {
  beforeEach(() => invokeMock.mockReset().mockResolvedValue(undefined));
  afterEach(cleanup);

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

  it("opens one Markdown source click through the native HTTP browser boundary", () => {
    const sourceUrl = "https://example.com/research";
    render(
      <ChatMessageContent
        content={`Read the [verified source](${sourceUrl}).`}
        role="assistant"
      />,
      { wrapper: I18nProvider },
    );

    fireEvent.click(screen.getByRole("link", { name: "verified source" }));
    expect(invokeMock).toHaveBeenCalledWith("open_external_http_url", { url: sourceUrl });
  });
});
