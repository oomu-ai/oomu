import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { BrowserModPanel } from "./BrowserModPanel";
import type { BrowserSplitRoute } from "./browserRouting";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauriRuntime: true,
}));

const route: BrowserSplitRoute = {
  messageId: 41,
  sessionId: "browser-contract-test",
  modId: "ai.eldris.mods.browser",
  action: "NAVIGATE",
  url: "https://example.com/private/path?token=hidden",
  reason: null,
  rawDirective: "",
};

function renderPanel(onResearchRouteUnavailable = vi.fn()) {
  return {
    onResearchRouteUnavailable,
    view: render(
      <BrowserModPanel
        onResearchRouteUnavailable={onResearchRouteUnavailable}
        route={route}
      />,
      { wrapper: I18nProvider },
    ),
  };
}

function authorization() {
  return {
    approvalToken: "approved-navigation-token",
    canonicalUrl: "https://example.com/private/path?token=hidden",
    canonicalOrigin: "https://example.com",
    destinationBinding: "bound-destination",
    expiresAtMs: Date.now() + 60_000,
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  vi.stubGlobal(
    "ResizeObserver",
    class ResizeObserver {
      observe() {}
      disconnect() {}
    },
  );
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callback(0);
    return 1;
  });
  vi.stubGlobal("cancelAnimationFrame", () => undefined);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("BrowserModPanel native bridge contract", () => {
  it("resumes an approved navigation after the Strict Mode effect replay", async () => {
    let resolveAuthorization: ((value: ReturnType<typeof authorization>) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "authorize_native_browser_navigation") {
        return new Promise<ReturnType<typeof authorization>>((resolve) => {
          resolveAuthorization = resolve;
        });
      }
      if (command === "open_authorized_native_browser") {
        return Promise.resolve({
          status: "ready",
          canonicalUrl: "https://example.com/private/path?token=hidden",
        });
      }
      return Promise.resolve(null);
    });

    render(
      <StrictMode>
        <BrowserModPanel route={route} />
      </StrictMode>,
      { wrapper: I18nProvider },
    );
    fireEvent.click(screen.getByRole("button", { name: "Open secure browser" }));
    expect(screen.getByRole("button", { name: "Waiting for approval…" })).toBeDisabled();

    await act(async () => {
      resolveAuthorization?.(authorization());
      await Promise.resolve();
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "open_authorized_native_browser",
        expect.objectContaining({ approvalToken: "approved-navigation-token" }),
      );
      expect(screen.getByText("example.com")).toBeInTheDocument();
    });
  });

  it("shows a completed native navigation only after the bridge acknowledges readiness", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "authorize_native_browser_navigation") return authorization();
      if (command === "open_authorized_native_browser") {
        return { status: "ready", canonicalUrl: "https://example.com/private/path?token=hidden" };
      }
      return null;
    });

    renderPanel();
    fireEvent.click(screen.getByRole("button", { name: "Open secure browser" }));

    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "Open secure browser" })).not.toBeInTheDocument();
      expect(screen.getByText("example.com")).toBeInTheDocument();
    });
    expect(invokeMock).toHaveBeenCalledWith("open_authorized_native_browser", expect.objectContaining({
      approvalToken: "approved-navigation-token",
    }));
    expect(screen.queryByText("Couldn't open the page")).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain("token=hidden");
  });

  it("treats a declined approval as a deliberate stop with no retry loop", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "authorize_native_browser_navigation") {
        throw { code: "browser_authorization_denied", message: "declined" };
      }
      return null;
    });

    const { onResearchRouteUnavailable } = renderPanel();
    fireEvent.click(screen.getByRole("button", { name: "Open secure browser" }));

    expect(await screen.findByText("Browser request stopped")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Try again" })).not.toBeInTheDocument();
    expect(invokeMock.mock.calls.map(([command]) => command)).not.toContain(
      "open_authorized_native_browser",
    );
    expect(onResearchRouteUnavailable).not.toHaveBeenCalled();
  });
});

describe("BrowserModPanel native bridge failure contract", () => {
  it("surfaces native policy rejection without exposing backend details", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "authorize_native_browser_navigation") return authorization();
      if (command === "open_authorized_native_browser") {
        throw JSON.stringify({
          code: "browser_navigation_blocked",
          message: "BACKEND CANARY: private-network-rule",
        });
      }
      return null;
    });

    const { onResearchRouteUnavailable } = renderPanel();
    fireEvent.click(screen.getByRole("button", { name: "Open secure browser" }));

    expect(await screen.findByText("Couldn't open the page")).toBeInTheDocument();
    expect(screen.getByText(
      "OOMU stopped before opening this page because the address didn't pass its safety check.",
    )).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("BACKEND CANARY");
    expect(onResearchRouteUnavailable).not.toHaveBeenCalled();
  });

  it("ends an unacknowledged native open after five seconds and closes partial browser state", async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation((command: string) => {
      if (command === "authorize_native_browser_navigation") {
        return Promise.resolve(authorization());
      }
      if (command === "open_authorized_native_browser") {
        return new Promise(() => undefined);
      }
      return Promise.resolve(null);
    });

    const { onResearchRouteUnavailable } = renderPanel();
    fireEvent.click(screen.getByRole("button", { name: "Open secure browser" }));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByRole("button", { name: "Opening securely…" })).toBeDisabled();

    await act(async () => {
      vi.advanceTimersByTime(5_000);
      await Promise.resolve();
    });

    expect(screen.getByText("Couldn't open the page")).toBeInTheDocument();
    expect(screen.getByText(
      "The secure browser didn't open in time. Nothing else happened. Try again.",
    )).toBeInTheDocument();
    expect(invokeMock.mock.calls.map(([command]) => command)).toContain("close_native_browser");
    expect(onResearchRouteUnavailable).toHaveBeenCalledTimes(1);
    expect(onResearchRouteUnavailable).toHaveBeenCalledWith(route, "timeout");
  });

  it("hands recoverable failures off once and removes the retry path", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "authorize_native_browser_navigation") return authorization();
      if (command === "open_authorized_native_browser") {
        throw { code: "browser_route_unavailable" };
      }
      return null;
    });
    const onResearchRouteUnavailable = vi.fn().mockResolvedValue(true);

    renderPanel(onResearchRouteUnavailable);
    fireEvent.click(screen.getByRole("button", { name: "Open secure browser" }));

    expect(await screen.findByText("Continuing in the background")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Try again" })).not.toBeInTheDocument();
    expect(onResearchRouteUnavailable).toHaveBeenCalledTimes(1);
    expect(onResearchRouteUnavailable).toHaveBeenCalledWith(route, "route_unavailable");
  });
});
