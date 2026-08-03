import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { ModelsScreen } from "../ModelsScreen";

const invokeMock = vi.hoisted(() => vi.fn());
const modelRoutingPreferencesMock = vi.hoisted(() => ({
  primaryRoute: null,
  fallbackRoute: null,
  setRoutePreference: vi.fn(),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
}));

vi.mock("@/app/hooks/useModelRoute", () => ({
  useModelRoutingPreferences: () => modelRoutingPreferencesMock,
}));

function renderModelsScreen() {
  return render(
    <ModelsScreen configuredProviders={[]} onConfiguredProvidersChange={vi.fn()} />,
    { wrapper: I18nProvider },
  );
}

function renderModelsScreenWithLegacyOpenAi() {
  return render(
    <ModelsScreen
      configuredProviders={[
        {
          id: "prov-legacy-openai",
          providerId: "openai",
          providerName: "Legacy OpenAI",
          authMethod: "api_key",
          baseUrl: "https://api.openai.com/v1",
          apiKeyLabel: "OPENAI_API_KEY",
          customModelIds: "gpt-4-turbo",
        },
      ]}
      onConfiguredProvidersChange={vi.fn()}
    />,
    { wrapper: I18nProvider },
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue([]);
});

afterEach(() => cleanup());

describe("ModelsScreen catalog", () => {
  it("offers only the supplied 2026 provider and model catalog", () => {
    renderModelsScreen();

    fireEvent.click(screen.getByRole("button", { name: /Add provider/ }));

    const catalogSelect = screen.getAllByRole("combobox")[0] as HTMLSelectElement;
    let modelSelect = screen.getAllByRole("combobox")[1];
    expect(within(modelSelect).getByRole("option", { name: /Gemma 4 E2B/ })).toBeTruthy();
    expect(within(modelSelect).getByRole("option", { name: /Gemma 4 E4B/ })).toBeTruthy();
    expect(within(modelSelect).getByRole("option", { name: /Gemma 4 12B/ })).toBeTruthy();

    fireEvent.change(catalogSelect, { target: { value: "google" } });
    modelSelect = screen.getAllByRole("combobox")[1];
    expect((modelSelect as HTMLSelectElement).value).toBe("gemini-3.6-flash");
    expect(within(modelSelect).getByRole("option", { name: /Gemini 3\.6 Flash/ })).toBeTruthy();
    expect(within(modelSelect).getByRole("option", { name: /^Google Gemini 3\.5 Flash$/ })).toBeTruthy();
    expect(within(modelSelect).getByRole("option", { name: /Gemini 3\.1 Pro Preview/ })).toBeTruthy();
    expect(within(modelSelect).queryByRole("option", { name: /Gemini 1\.5/ })).toBeNull();

    fireEvent.change(catalogSelect, { target: { value: "openai" } });
    modelSelect = screen.getAllByRole("combobox")[1];
    expect((modelSelect as HTMLSelectElement).value).toBe("gpt-5.6-sol");
    expect(within(modelSelect).getByRole("option", { name: /GPT-5\.6 Sol/ })).toBeTruthy();
    expect(within(modelSelect).getByRole("option", { name: /GPT-5\.5/ })).toBeTruthy();

    fireEvent.change(catalogSelect, { target: { value: "anthropic" } });
    modelSelect = screen.getAllByRole("combobox")[1];
    expect((modelSelect as HTMLSelectElement).value).toBe("claude-fable-5");
    expect(within(modelSelect).getByRole("option", { name: /Claude Fable 5/ })).toBeTruthy();
    expect(within(modelSelect).queryByRole("option", { name: /Claude 3\.5/ })).toBeNull();

    expect(within(catalogSelect).getByRole("option", { name: /DeepSeek Direct/ })).toBeTruthy();
    expect(within(catalogSelect).getByRole("option", { name: /Qwen.*United States/ })).toBeTruthy();
    expect(within(catalogSelect).getByRole("option", { name: /Qwen.*International/ })).toBeTruthy();
    expect(within(catalogSelect).getByRole("option", { name: /Z\.AI GLM API.*Global/ })).toBeTruthy();
    expect(within(catalogSelect).getByRole("option", { name: /Z\.AI GLM Coding Plan.*Global/ })).toBeTruthy();
    expect(within(catalogSelect).getByRole("option", { name: /Zhipu AI GLM Direct/ })).toBeTruthy();
    expect(within(catalogSelect).getByRole("option", { name: /Moonshot AI.*Global/ })).toBeTruthy();
    expect(within(catalogSelect).getByRole("option", { name: /Moonshot AI.*Mainland China/ })).toBeTruthy();
    expect(within(catalogSelect).queryByRole("option", { name: /Mistral/ })).toBeNull();
    expect(within(catalogSelect).queryByRole("option", { name: /Together/ })).toBeNull();

    fireEvent.change(catalogSelect, { target: { value: "zai" } });
    modelSelect = screen.getAllByRole("combobox")[1];
    expect((modelSelect as HTMLSelectElement).value).toBe("zai/glm-5.2");
    expect(within(modelSelect).getByRole("option", { name: /^GLM-5\.2$/ })).toBeTruthy();
  });
});

describe("ModelsScreen provider configuration", () => {
  it("shows an incomplete cloud provider honestly and keeps it out of route assignment", () => {
    render(
      <ModelsScreen
        configuredProviders={[
          {
            id: "missing-key", providerId: "openai", providerName: "OpenAI without a key",
            authMethod: "api_key", baseUrl: "https://api.openai.com/v1",
            apiKeyLabel: "OPENAI_API_KEY", customModelIds: "gpt-5.6-sol",
            credentialConfigured: false, autoRouteTarget: true,
          },
          {
            id: "local-ready", providerId: "local_model", providerName: "On this Mac",
            authMethod: "custom", baseUrl: "", apiKeyLabel: "",
            customModelIds: "gemma-4-E4B-it-qat-q4_0-gguf", credentialConfigured: false,
          },
        ]}
        onConfiguredProvidersChange={vi.fn()}
      />,
      { wrapper: I18nProvider },
    );

    expect(screen.getAllByText("API key needed")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: /OpenAI without a key/ }));
    expect(screen.getByText("Add this provider’s API key before using it in a chat.")).toBeInTheDocument();
    for (const checkbox of screen.getAllByRole("checkbox")) {
      expect(checkbox).toBeDisabled();
    }
  });

  it("keeps catalog endpoints canonical while leaving custom endpoints editable", () => {
    renderModelsScreen();
    fireEvent.click(screen.getByRole("button", { name: /Add provider/ }));

    const catalogSelect = screen.getAllByRole("combobox")[0];
    fireEvent.change(catalogSelect, { target: { value: "openai" } });
    fireEvent.click(screen.getByRole("button", { name: /Show advanced configuration/ }));

    const canonicalUrl = screen.getByLabelText("Provider address") as HTMLInputElement;
    expect(canonicalUrl).toHaveValue("https://api.openai.com/v1");
    expect(canonicalUrl).toHaveAttribute("readonly");
    expect(screen.getByText(/OOMU manages this address/)).toBeInTheDocument();
    expect(screen.queryByText("Manual model IDs")).toBeNull();

    fireEvent.change(catalogSelect, { target: { value: "custom" } });
    const customUrl = screen.getByLabelText("Base URL override") as HTMLInputElement;
    expect(customUrl).not.toHaveAttribute("readonly");
    fireEvent.change(customUrl, { target: { value: "https://models.example.test/v1" } });
    expect(customUrl).toHaveValue("https://models.example.test/v1");
    expect(screen.getByText(/Manual model IDs/)).toBeInTheDocument();
  });

  it("preserves a saved out-of-catalog model as a disabled migration choice", () => {
    renderModelsScreenWithLegacyOpenAi();

    fireEvent.click(screen.getByRole("button", { name: /Legacy OpenAI/ }));
    const legacyOption = screen.getByRole("option", {
      name: /gpt-4-turbo.*Saved legacy model/,
    }) as HTMLOptionElement;

    expect(legacyOption.disabled).toBe(true);
    expect(
      screen.getByText(/This saved model is no longer in OOMU’s current catalog/),
    ).toBeInTheDocument();
  });
});

describe("ModelsScreen credential hygiene", () => {
  it("clears provider credential drafts from state, DOM, errors, and unmount paths", async () => {
    const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "save_provider_config") {
        throw new Error("token=server-error-canary");
      }
      return [];
    });
    const view = renderModelsScreen();
    const rendered = within(view.container);
    fireEvent.click(rendered.getByRole("button", { name: /Add provider/ }));
    fireEvent.change(rendered.getAllByRole("combobox")[0], { target: { value: "openai" } });
    const keyInput = rendered.getByPlaceholderText("Paste your API key") as HTMLInputElement;
    fireEvent.change(keyInput, { target: { value: "provider-draft-canary" } });
    fireEvent.click(rendered.getByRole("button", { name: "Save Configuration" }));

    await waitFor(() => expect(keyInput).toHaveValue(""));
    expect(JSON.stringify(consoleSpy.mock.calls)).not.toContain("server-error-canary");

    fireEvent.change(keyInput, { target: { value: "unmount-draft-canary" } });
    view.unmount();
    expect(keyInput.value).toBe("");
    consoleSpy.mockRestore();
  });
});
