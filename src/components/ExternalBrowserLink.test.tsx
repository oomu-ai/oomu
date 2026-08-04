import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ExternalBrowserLink, safeExternalHttpUrl } from "./ExternalBrowserLink";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));

describe("ExternalBrowserLink", () => {
  beforeEach(() => invokeMock.mockReset().mockResolvedValue(undefined));
  afterEach(cleanup);

  it("requests exactly one native browser open for one HTTPS click", () => {
    render(<ExternalBrowserLink href="https://example.com/source?q=oomu">Source</ExternalBrowserLink>);
    fireEvent.click(screen.getByRole("link", { name: "Source" }));
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("open_external_http_url", {
      url: "https://example.com/source?q=oomu",
    });
  });

  it("allows HTTP while keeping native policy authoritative", () => {
    render(<ExternalBrowserLink href="http://example.com/">HTTP Source</ExternalBrowserLink>);
    fireEvent.click(screen.getByRole("link", { name: "HTTP Source" }));
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it("renders unsafe schemes and credential-bearing URLs as inert text", () => {
    for (const href of [
      "javascript:alert(1)",
      "file:///tmp/private",
      "mailto:user@example.com",
      "https://user:secret@example.com/",
    ]) {
      const { unmount } = render(<ExternalBrowserLink href={href}>Unsafe</ExternalBrowserLink>);
      expect(screen.queryByRole("link", { name: "Unsafe" })).not.toBeInTheDocument();
      unmount();
    }
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("absorbs native failures without an unhandled rejection", async () => {
    invokeMock.mockRejectedValueOnce(new Error("external_url_open_failed"));
    render(<ExternalBrowserLink href="https://example.com/">Failure Source</ExternalBrowserLink>);
    fireEvent.click(screen.getByRole("link", { name: "Failure Source" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(1));
  });
});

describe("safeExternalHttpUrl", () => {
  it("rejects malformed and padded URLs", () => {
    expect(safeExternalHttpUrl(" https://example.com/")).toBeNull();
    expect(safeExternalHttpUrl("https:/// ")).toBeNull();
    expect(safeExternalHttpUrl(undefined)).toBeNull();
  });
});
