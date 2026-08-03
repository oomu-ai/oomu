import { render, screen } from "@testing-library/react";
import { I18nProvider } from "@/context/I18nContext";
import { describe, expect, it } from "vitest";
import {
  compactExecutionModelLabel,
  modelIdentityIsOpaque,
  RoutingIndicator,
} from "./RoutingIndicator";

describe("RoutingIndicator", () => {
  it("keeps model labels compact and recognizable", () => {
    expect(compactExecutionModelLabel("google/gemma-4-27b")).toBe("Gemma 4 27B");
    expect(compactExecutionModelLabel("gemma-4-12B-it-qat-q4_0-gguf")).toBe("Gemma 4 12B");
    expect(compactExecutionModelLabel("google/gemini-3.5-flash")).toBe("Gemini 3.5 Flash");
    expect(compactExecutionModelLabel("openai:gpt-5.2")).toBe("GPT 5.2");
    expect(compactExecutionModelLabel("gemma-4-E2B-it-qat-q4_0-gguf")).toBe("Gemma 4 E2B");
  });

  it("never presents a storage directory or path as a model identity", () => {
    expect(modelIdentityIsOpaque("models")).toBe(true);
    expect(modelIdentityIsOpaque("/Users/test/Models/gemma-4-E2B.gguf")).toBe(true);
    expect(compactExecutionModelLabel("models", "On-device model")).toBe("On-device model");
    expect(compactExecutionModelLabel(
      "/Users/test/Models/gemma-4-E2B.gguf",
      "On-device model",
    )).toBe("On-device model");
  });

  it("shows the authoritative local and cloud pair without promising an executor", () => {
    render(<RoutingIndicator
      autoRouteStatus="ready"
      cloudModelId="gemini-3.5-flash"
      isLocal={false}
      localModelId="gemma-4-12B-it-qat-q4_0-gguf"
      mode="auto"
      modelId=""
    />, {
      wrapper: I18nProvider,
    });
    expect(screen.getByLabelText("Auto-route ready. Local: Gemma 4 12B. Cloud: Gemini 3.5 Flash.")).toHaveTextContent(
      "Auto-route ready · Local: Gemma 4 12B · Cloud: Gemini 3.5 Flash",
    );
    expect(screen.queryByText(/Sovereign \(|Cloud \(/)).not.toBeInTheDocument();
  });

  it("makes degraded classifier state explicit", () => {
    render(<RoutingIndicator
      autoRouteStatus="degraded"
      isLocal
      localModelId="gemma-4-12B-it-qat-q4_0-gguf"
      mode="auto"
      modelId=""
    />, { wrapper: I18nProvider });
    expect(screen.getByText(/Auto-route needs attention/)).toBeInTheDocument();
    expect(screen.getByTitle("Choose a model or review the saved request to continue."))
      .toBeInTheDocument();
  });
});
