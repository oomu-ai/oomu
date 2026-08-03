import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "@/context/I18nContext";
import { SetupJourney } from "../integrations/SetupJourney";
import type {
  ConnectorAccount,
  ConnectorManifest,
  SetupState,
} from "../integrations/integrationClient";

const invokeMock = vi.hoisted(() => vi.fn());
const installEventListeners = vi.hoisted(
  () => new Map<string, (event: { payload: unknown }) => void>(),
);

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    installEventListeners.set(name, handler);
    return () => installEventListeners.delete(name);
  }),
}));

function localeState(
  activeLocale = "en-US",
  translations: Record<string, unknown> = {},
) {
  return {
    activeLocale,
    availableLocales: [
      {
        id: "en-US",
        label: "English (US)",
        fileName: "en-US.json",
        isDefault: true,
        verified: true,
      },
      ...(activeLocale === "en-US"
        ? []
        : [
            {
              id: activeLocale,
              label: activeLocale,
              fileName: `${activeLocale}.json`,
              isDefault: false,
              verified: true,
            },
          ]),
    ],
    translations,
  };
}

function googleManifest(supported: boolean): ConnectorManifest {
  return {
    manifestId: "google_workspace",
    name: "Google Workspace",
    version: 1,
    transport: "https_api",
    authMethod: "oauth_authorization_code_pkce",
    tools: [],
    requestedPermissions: ["Read Gmail", "Read Calendar"],
    dataDestinations: ["https://accounts.google.com"],
    projectEligible: true,
    supported,
    availabilityReasonCode: supported ? undefined : "build_missing_oauth_client",
  };
}

function microsoftManifest(supported: boolean): ConnectorManifest {
  return {
    manifestId: "microsoft_365",
    name: "BACKEND MICROSOFT CANARY",
    version: 1,
    transport: "https_api",
    authMethod: "oauth_authorization_code_pkce",
    tools: [],
    requestedPermissions: ["BACKEND PERMISSION CANARY"],
    dataDestinations: ["https://login.microsoftonline.com", "https://graph.microsoft.com"],
    projectEligible: true,
    supported,
    baseScopes: ["openid", "profile", "email", "offline_access", "User.Read"],
  };
}

const authorizedGoogle: ConnectorAccount = {
  connectorId: "connector_00000000-0000-4000-8000-000000000123",
  manifestId: "google_workspace",
  accountLabel: "user@example.com",
  grantedScopes: ["openid"],
  connectionState: "authorized",
  schemaVersion: 1,
  allProjectsEnabled: false,
  projectScopeReviewedAtMs: 1,
  enabledProjectIds: [],
};

const readyLocalModels = [{ id: "gemma-ready.gguf", compatibility: "ready" }];
const e2bModelId = "gemma-4-E2B-it-qat-q4_0-gguf";
const e4bModelId = "gemma-4-E4B-it-qat-q4_0-gguf";

function renderJourney(
  initialState: SetupState,
  onComplete = vi.fn(),
  previewMode = false,
) {
  return render(
    <SetupJourney
      initialState={initialState}
      onComplete={onComplete}
      previewMode={previewMode}
    />,
    { wrapper: I18nProvider },
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  installEventListeners.clear();
});
afterEach(cleanup);

describe("SetupJourney initial local model persistence", () => {
  it("persists the clean E2B choice and a usable local provider before advancing", async () => {
    const onProviderConfigured = vi.fn();
    const configuredProvider = {
      id: "local-model",
      providerId: "local_model",
      providerName: "On-device model",
      authMethod: "custom" as const,
      baseUrl: "",
      apiKeyLabel: "",
      customModelIds: `${e2bModelId}\n${e4bModelId}`,
      autoRouteTarget: false,
      createdAtMs: 1,
      updatedAtMs: 1,
    };
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_local_models") {
        return [
          { id: e4bModelId, compatibility: "ready" },
          { id: e2bModelId, compatibility: "ready" },
        ];
      }
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "get_default_prewarmed_model") {
        return { modelId: e2bModelId, isDefault: true };
      }
      if (command === "set_default_prewarmed_model") {
        return { modelId: e2bModelId, isDefault: false };
      }
      if (command === "save_provider_config") {
        expect(args).toEqual({
          request: expect.objectContaining({
            id: "local-model",
            providerId: "local_model",
            customModelIds: `${e2bModelId}\n${e4bModelId}`,
          }),
        });
        return configuredProvider;
      }
      if (command === "save_setup_progress") {
        return { currentStep: "permissions", ...(args?.request as object) };
      }
      return null;
    });

    render(
      <SetupJourney
        initialState={{ currentStep: "model" }}
        onComplete={vi.fn()}
        onProviderConfigured={onProviderConfigured}
      />,
      { wrapper: I18nProvider },
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("list_local_models"),
    );
    fireEvent.click(screen.getByRole("button", { name: "Use existing model and continue" }));

    await waitFor(() => expect(onProviderConfigured).toHaveBeenCalledWith(configuredProvider));
    expect(invokeMock).toHaveBeenCalledWith("set_default_prewarmed_model", {
      modelId: e2bModelId,
      model_id: e2bModelId,
    });
    const commands = invokeMock.mock.calls.map(([command]) => command);
    expect(commands.indexOf("save_provider_config")).toBeLessThan(
      commands.indexOf("save_setup_progress"),
    );
  });
});

describe("SetupJourney local model folder refresh", () => {
  it("refreshes a newly chosen model folder and rechecks it before advancing", async () => {
    let folderSelected = false;
    let modelInventoryReads = 0;
    const configuredProvider = {
      id: "local-model",
      providerId: "local_model",
      providerName: "On-device model",
      authMethod: "custom" as const,
      baseUrl: "",
      apiKeyLabel: "",
      customModelIds: e2bModelId,
      autoRouteTarget: false,
      createdAtMs: 1,
      updatedAtMs: 1,
    };
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_local_models") {
        modelInventoryReads += 1;
        return folderSelected
          ? [{ id: e2bModelId, compatibility: "ready" }]
          : [];
      }
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "choose_local_model_directory") {
        folderSelected = true;
        return { path: `/models/${e2bModelId}`, isDefault: false };
      }
      if (command === "get_default_prewarmed_model") {
        return { modelId: e2bModelId, isDefault: true };
      }
      if (command === "set_default_prewarmed_model") {
        return { modelId: e2bModelId, isDefault: false };
      }
      if (command === "save_provider_config") return configuredProvider;
      if (command === "save_setup_progress") {
        return { currentStep: "permissions", ...(args?.request as object) };
      }
      return null;
    });

    renderJourney({ currentStep: "model" });
    await waitFor(() => expect(modelInventoryReads).toBe(1));

    fireEvent.click(screen.getByRole("button", { name: "Choose an existing model folder…" }));
    await waitFor(() => expect(modelInventoryReads).toBe(2));

    fireEvent.click(screen.getByRole("button", { name: "Use existing model and continue" }));

    expect(await screen.findByText("Step 2 of 4")).toBeVisible();
    expect(modelInventoryReads).toBe(3);
    expect(invokeMock).toHaveBeenCalledWith("set_default_prewarmed_model", {
      modelId: e2bModelId,
      model_id: e2bModelId,
    });
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps the current inventory when model-folder selection is canceled", async () => {
    let modelInventoryReads = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_local_models") {
        modelInventoryReads += 1;
        return readyLocalModels;
      }
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "choose_local_model_directory") return null;
      return null;
    });

    renderJourney({ currentStep: "model" });
    await waitFor(() => expect(modelInventoryReads).toBe(1));

    const chooseButton = screen.getByRole("button", { name: "Choose an existing model folder…" });
    fireEvent.click(chooseButton);

    await waitFor(() => expect(chooseButton).not.toBeDisabled());
    expect(modelInventoryReads).toBe(1);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("SetupJourney recommended model", () => {
  it("always lets a user continue without setting up a model", async () => {
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_recommended_model_install_state") {
        return { packageState: "absent", activeInstall: null };
      }
      if (command === "list_provider_configs") return [];
      if (command === "list_local_models") return [];
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "save_setup_progress") {
        return { currentStep: "permissions", ...(args?.request as object) };
      }
      return null;
    });

    renderJourney({ currentStep: "model" });
    const later = await screen.findByRole("button", { name: "Set up later" });
    expect(later).toBeEnabled();
    fireEvent.click(later);

    expect(await screen.findByText("Step 2 of 4")).toBeVisible();
    expect(invokeMock.mock.calls.some(([command]) => command === "save_provider_config"))
      .toBe(false);
  });

});

describe("SetupJourney recommended model activation", () => {

  it("advances only after native evidence and exact-model inventory agree", async () => {
    let installed = false;
    const onProviderConfigured = vi.fn();
    const configuredProvider = {
      id: "local-model",
      providerId: "local_model",
      providerName: "On-device model",
      authMethod: "custom" as const,
      baseUrl: "",
      apiKeyLabel: "",
      customModelIds: e2bModelId,
      autoRouteTarget: false,
      createdAtMs: 1,
      updatedAtMs: 1,
    };
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_recommended_model_install_state") {
        return { packageState: "absent", activeInstall: null };
      }
      if (command === "list_provider_configs") return installed ? [configuredProvider] : [];
      if (command === "list_local_models") {
        return installed ? [{ id: e2bModelId, compatibility: "ready" }] : [];
      }
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "start_recommended_model_install") {
        return {
          installId: "install-1",
          attached: false,
          progress: {
            installId: "install-1",
            state: "downloading",
            downloadedBytes: 0,
            totalBytes: 4_336_349_920,
            canCancel: true,
            canResume: false,
          },
        };
      }
      if (command === "run_setup_sample_task") {
        return {
          currentStep: "model",
          modelPath: "local",
          completionChannel: "local",
          sampleProjectId: "project_recommended_sample",
        };
      }
      if (command === "save_setup_progress") {
        return { currentStep: "permissions", ...(args?.request as object) };
      }
      return null;
    });

    render(
      <SetupJourney
        initialState={{ currentStep: "model" }}
        onComplete={vi.fn()}
        onProviderConfigured={onProviderConfigured}
      />,
      { wrapper: I18nProvider },
    );
    fireEvent.click(await screen.findByRole("button", { name: "Download and continue" }));
    await waitFor(() => expect(screen.getByRole("progressbar")).toBeVisible());
    expect(invokeMock.mock.calls.some(([command]) => command === "save_setup_progress"))
      .toBe(false);

    installed = true;
    await act(async () => {
      installEventListeners.get("recommended-model-install-progress")?.({
        payload: {
          installId: "install-1",
          state: "ready",
          downloadedBytes: 4_336_349_920,
          totalBytes: 4_336_349_920,
          canCancel: false,
          canResume: false,
          completedProvider: {
            providerId: "local-model",
            providerType: "local_model",
            modelId: e2bModelId,
            verified: true,
          },
        },
      });
    });

    expect(await screen.findByText("Step 2 of 4")).toBeVisible();
    expect(invokeMock).not.toHaveBeenCalledWith("run_setup_sample_task", expect.anything());
    expect(onProviderConfigured).toHaveBeenCalledWith(configuredProvider);
  });
});

describe("SetupJourney connector setup", () => {
  it("explains an unavailable connector instead of showing a dead Connect button", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_connector_manifests") return [googleManifest(false)];
      if (command === "list_connector_accounts") return [];
      return null;
    });

    renderJourney({ currentStep: "connectors" });

    expect(await screen.findByText("Google Workspace")).toBeVisible();
    expect(screen.getByText("Google Workspace isn’t available in this version.")).toBeVisible();
    expect(screen.getByText(/There’s nothing to configure here/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Connect" })).toBeNull();
    expect(screen.getByText("Unavailable")).toBeVisible();
  });

  it("defers Microsoft setup to the exact-consent Integrations flow", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_connector_manifests") return [microsoftManifest(true)];
      if (command === "list_connector_accounts") return [];
      return null;
    });

    renderJourney({ currentStep: "connectors" });
    expect(await screen.findByText("Microsoft 365")).toBeVisible();
    expect(screen.getByText("Set up later in Connections")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Connect" })).toBeNull();
    expect(invokeMock.mock.calls.filter(([command]) => command === "begin_connector_oauth")).toHaveLength(0);
  });
});

describe("SetupJourney OAuth status recovery", () => {
  it("tracks OAuth completion and prevents duplicate connection attempts", async () => {
    let connected = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_connector_manifests") return [googleManifest(true)];
      if (command === "list_connector_accounts") return connected ? [authorizedGoogle] : [];
      if (command === "begin_connector_oauth") {
        return {
          connectorId: authorizedGoogle.connectorId,
          authorizationUrl: "https://accounts.google.com/o/oauth2/v2/auth",
          expiresAtMs: Date.now() + 60_000,
        };
      }
      if (command === "get_connector_connection_status") {
        connected = true;
        return {
          connectorId: authorizedGoogle.connectorId,
          connectionState: "authorized",
          grantedScopes: authorizedGoogle.grantedScopes,
          lastProbeCode: "oauth_completed",
        };
      }
      return null;
    });

    renderJourney({ currentStep: "connectors" });
    const connect = await screen.findByRole("button", { name: "Connect" });
    fireEvent.click(connect);

    await waitFor(() => expect(connect).toBeDisabled());
    expect(screen.getByRole("button", { name: "Finishing connection" })).toBeDisabled();
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "begin_connector_oauth"),
    ).toHaveLength(1);

    expect(await screen.findByText("user@example.com", {}, { timeout: 2_500 })).toBeVisible();
    expect(invokeMock).toHaveBeenCalledWith("get_connector_connection_status", {
      request: { connectorId: authorizedGoogle.connectorId },
    });
    expect(screen.queryByRole("button", { name: "Connect" })).toBeNull();
  });

  it("localizes a fresh Google OAuth wrong-client failure by its exact connector status", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") {
        return localeState("fr-FR", {
          setup: {
            connector_google_name: "Espace de travail Google",
            connector_google_permissions: "Lire Gmail et Agenda",
            connector_connect: "Connecter",
            connector_connecting: "Connexion en cours",
            errors: {
              setup_google_oauth_configuration_failed:
                "Google a refusé la configuration OAuth de bureau d’OOMU.",
            },
          },
        });
      }
      if (command === "list_provider_configs") return [];
      if (command === "list_connector_manifests") return [googleManifest(true)];
      if (command === "list_connector_accounts") return [];
      if (command === "begin_connector_oauth") {
        return {
          connectorId: authorizedGoogle.connectorId,
          authorizationUrl: "https://accounts.google.com/o/oauth2/v2/auth",
          expiresAtMs: Date.now() + 60_000,
        };
      }
      if (command === "get_connector_connection_status") {
        return {
          connectorId: authorizedGoogle.connectorId,
          connectionState: "disconnected",
          grantedScopes: [],
          lastProbeCode: "google_token_client_authentication_required",
        };
      }
      return null;
    });

    renderJourney({ currentStep: "connectors" });
    fireEvent.click(await screen.findByRole("button", { name: "Connecter" }));

    expect(await screen.findByRole("alert", {}, { timeout: 2_500 })).toHaveTextContent(
      "Google a refusé la configuration OAuth de bureau d’OOMU.",
    );
    expect(invokeMock).toHaveBeenCalledWith("get_connector_connection_status", {
      request: { connectorId: authorizedGoogle.connectorId },
    });
    expect(screen.getByRole("button", { name: "Connecter" })).toBeEnabled();
  });

  it("removes the dead delivery step and resumes its legacy state at the real sample", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      return null;
    });

    renderJourney({ currentStep: "channel", completionChannel: "telegram" });

    expect(await screen.findByText("Step 4 of 4")).toBeVisible();
    expect(screen.getByRole("heading", { name: "See it work" })).toBeVisible();
    expect(screen.queryByRole("combobox")).toBeNull();
  });
});

describe("SetupJourney localized recovery", () => {
  it("persists local notices and localizes a structured sample error", async () => {
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_local_models") return readyLocalModels;
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "save_setup_progress") {
        return { currentStep: "sample", ...(args?.request as object) };
      }
      if (command === "run_setup_sample_task") {
        throw Object.assign(new Error("raw backend detail"), {
          code: "setup_model_output_empty",
        });
      }
      return null;
    });

    renderJourney({ currentStep: "connectors", modelPath: "local" });
    fireEvent.click(await screen.findByRole("button", { name: "Continue" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("save_setup_progress", {
        request: {
          currentStep: "sample",
          modelPath: "local",
          completionChannel: "local",
        },
      }),
    );
    fireEvent.click(await screen.findByRole("button", { name: "Try it out" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The selected model returned no visible output after verified retries. Try again.",
    );
    expect(screen.getByRole("alert")).not.toHaveTextContent("raw backend detail");
  });

  it("uses translated connector copy with the safe English fallback for new availability text", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") {
        return localeState("fr-FR", {
          setup: {
            connectors_title: "Connectez votre travail",
            connectors_help: "Ajoutez un service maintenant ou plus tard.",
            connector_google_name: "Espace de travail Google",
            connector_google_permissions: "Lire Gmail et Agenda",
            connector_google_unavailable: "Cette version ne permet pas Google OAuth.",
            connector_unavailable: "Indisponible",
          },
        });
      }
      if (command === "list_provider_configs") return [];
      if (command === "list_connector_manifests") return [googleManifest(false)];
      if (command === "list_connector_accounts") return [];
      return null;
    });

    renderJourney({ currentStep: "connectors" });

    expect(await screen.findByText("Espace de travail Google")).toBeVisible();
    expect(screen.getByText("Lire Gmail et Agenda")).toBeVisible();
    expect(screen.getByText("Espace de travail Google isn’t available in this version.")).toBeVisible();
    expect(
      screen.queryByText("This build does not include the official Google OAuth client identity."),
    ).toBeNull();
  });

  it("uses the active locale for structured native errors", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") {
        return localeState("fr-FR", {
          setup: {
            sample_title: "Exécuter un exemple réel",
            sample_help: "Vérifiez le modèle avec une tâche réelle.",
            sample_name: "Premier projet OOMU",
            run_sample: "Exécuter l’exemple vérifié",
            errors: {
              setup_model_output_empty:
                "Le modèle n’a renvoyé aucun résultat visible. Réessayez.",
            },
          },
        });
      }
      if (command === "list_provider_configs") return [];
      if (command === "list_local_models") return readyLocalModels;
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "run_setup_sample_task") {
        throw Object.assign(new Error("raw backend detail"), {
          code: "setup_model_output_empty",
        });
      }
      return null;
    });

    renderJourney({ currentStep: "sample", modelPath: "local" });
    fireEvent.click(
      await screen.findByRole("button", { name: "Exécuter l’exemple vérifié" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Le modèle n’a renvoyé aucun résultat visible. Réessayez.",
    );
    expect(screen.getByRole("alert")).not.toHaveTextContent("raw backend detail");
  });
});

describe("SetupJourney first-run preview", () => {
  it("keeps preview navigation out of durable setup state", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "get_recommended_model_install_state") {
        return { packageState: "absent", activeInstall: null };
      }
      if (command === "list_provider_configs") return [];
      if (command === "list_local_models") return [];
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      return null;
    });

    renderJourney({ currentStep: "model" }, vi.fn(), true);
    fireEvent.click(await screen.findByRole("button", { name: "Set up later" }));

    expect(await screen.findByText("Step 2 of 4")).toBeVisible();
    expect(invokeMock.mock.calls.some(([command]) => command === "save_setup_progress"))
      .toBe(false);
  });

  it("runs a real sample without completing durable setup in preview mode", async () => {
    const onComplete = vi.fn();
    const sampleResult: SetupState = {
      currentStep: "sample",
      modelPath: "local",
      sampleProjectId: "project_preview_verified",
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_local_models") return readyLocalModels;
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "run_setup_sample_task") return sampleResult;
      return null;
    });

    renderJourney({ currentStep: "sample", modelPath: "local" }, onComplete, true);
    fireEvent.click(await screen.findByRole("button", { name: "Try it out" }));

    await waitFor(() => expect(onComplete).toHaveBeenCalledWith(sampleResult));
    expect(invokeMock).toHaveBeenCalledWith("run_setup_sample_task", {
      request: { modelRoute: "local", completeSetup: false },
    });
    expect(invokeMock.mock.calls.some(([command]) => command === "save_setup_progress"))
      .toBe(false);
  });
});

describe("SetupJourney completion", () => {
  it("does not repeat a verified recommended-model sample before entering OOMU", async () => {
    const onComplete = vi.fn();
    const finished: SetupState = {
      currentStep: "finished",
      modelPath: "local",
      completionChannel: "local",
      sampleProjectId: "project_recommended_sample",
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_local_models") return readyLocalModels;
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "save_setup_progress") return finished;
      return null;
    });

    renderJourney({
      currentStep: "sample",
      modelPath: "local",
      sampleProjectId: "project_recommended_sample",
    }, onComplete);

    expect(await screen.findByText("Ready")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Try it out" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    await waitFor(() => expect(onComplete).toHaveBeenCalledWith(finished));
    expect(invokeMock.mock.calls.some(([command]) => command === "run_setup_sample_task"))
      .toBe(false);
  });

  it("explains how to recover when the real sample cannot use durable storage", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_local_models") return readyLocalModels;
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "run_setup_sample_task") {
        throw Object.assign(new Error("volatile database path was private"), {
          code: "setup_storage_recovery_required",
        });
      }
      return null;
    });

    renderJourney({ currentStep: "sample", modelPath: "local" });
    fireEvent.click(await screen.findByRole("button", { name: "Try it out" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "OOMU needs to restore its secure storage first. Restart OOMU, finish recovery, then try again.",
    );
    expect(screen.getByRole("alert")).not.toHaveTextContent("private");
  });

  it("hands a verified finished state to the durable access gate", async () => {
    const onComplete = vi.fn();
    const finished: SetupState = {
      currentStep: "finished",
      modelPath: "local",
      completionChannel: "local",
      sampleProjectId: "project_verified",
      completedAtMs: 1_725_000_000_000,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_local_models") return readyLocalModels;
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "run_setup_sample_task") return finished;
      return null;
    });

    renderJourney({ currentStep: "sample", modelPath: "local" }, onComplete);
    fireEvent.click(await screen.findByRole("button", { name: "Try it out" }));

    await waitFor(() => expect(onComplete).toHaveBeenCalledWith(finished));
    expect(onComplete).toHaveBeenCalledTimes(1);
  });

  it("lets the user skip the optional sample and enter OOMU", async () => {
    const onComplete = vi.fn();
    const finished: SetupState = {
      currentStep: "finished",
      modelPath: "local",
      completionChannel: "local",
    };
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "save_setup_progress") {
        expect(args).toEqual({
          request: {
            currentStep: "finished",
            modelPath: "local",
            completionChannel: "local",
          },
        });
        return finished;
      }
      return null;
    });

    renderJourney({ currentStep: "sample", modelPath: "local" }, onComplete);
    expect(await screen.findByText(/No runnable model is connected yet/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Try it out" })).toBeNull();
    fireEvent.click(
      await screen.findByRole("button", { name: "Skip and start using OOMU" }),
    );

    await waitFor(() => expect(onComplete).toHaveBeenCalledWith(finished));
    expect(onComplete).toHaveBeenCalledTimes(1);
  });
});

describe("SetupJourney nonblocking sample escape", () => {
  it("never blocks the skip path on a stalled model inventory probe", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_local_models") return new Promise(() => undefined);
      if (command === "list_provider_configs") return [];
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      return null;
    });

    renderJourney({ currentStep: "sample", modelPath: "local" });

    expect(
      await screen.findByRole("button", { name: "Skip and start using OOMU" }),
    ).toBeEnabled();
  });

  it("keeps the skip path available after the sample model fails", async () => {
    const onComplete = vi.fn();
    const finished: SetupState = {
      currentStep: "finished",
      modelPath: "local",
      completionChannel: "local",
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_locale_state") return localeState();
      if (command === "list_provider_configs") return [];
      if (command === "list_local_models") return readyLocalModels;
      if (command === "list_connector_manifests") return [];
      if (command === "list_connector_accounts") return [];
      if (command === "run_setup_sample_task") {
        throw Object.assign(new Error("raw backend detail"), {
          code: "setup_model_execution_failed",
        });
      }
      if (command === "save_setup_progress") return finished;
      return null;
    });

    renderJourney({ currentStep: "sample", modelPath: "local" }, onComplete);
    fireEvent.click(await screen.findByRole("button", { name: "Try it out" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The selected model could not run the verified sample.",
    );

    fireEvent.click(screen.getByRole("button", { name: "Skip and start using OOMU" }));
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith(finished));
  });
});
