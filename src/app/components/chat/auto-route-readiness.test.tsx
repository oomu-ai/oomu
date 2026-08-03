import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { RoutingIndicator } from "./RoutingIndicator";

describe("auto-route-readiness", () => {
  it.each(["loading", "recovering"] as const)(
    "presents %s classifier health as preparation",
    (autoRouteStatus) => {
      const view = render(
        <RoutingIndicator
          autoRouteStatus={autoRouteStatus}
          cloudModelId="gemini-3.5-flash"
          isLocal
          localModelId="gemma-4-12B-it-qat-q4_0-gguf"
          mode="auto"
          modelId=""
        />,
        { wrapper: I18nProvider },
      );
      expect(within(view.container).getByText(/Preparing Auto-route/)).toBeInTheDocument();
    },
  );

  it("explains on-device routing without exposing internal readiness details", () => {
    render(
      <RoutingIndicator
        autoRouteStatus="ready"
        classifierModelId="gemma-4-E2B-it-qat-q4_0-gguf"
        cloudModelId="gemini-3.5-flash"
        isLocal
        localModelId="gemma-4-12B-it-qat-q4_0-gguf"
        mode="auto"
        modelId=""
      />,
      { wrapper: I18nProvider },
    );
    const indicator = screen.getByLabelText(
      "Auto-route ready. Local: Gemma 4 12B. Cloud: Gemini 3.5 Flash.",
    );
    expect(indicator).toHaveAttribute("tabindex", "0");
    expect(indicator).toHaveAttribute(
      "aria-description",
      "OOMU is ready to choose between the models shown.",
    );
    expect(indicator).not.toHaveTextContent("E2B");
  });
});
