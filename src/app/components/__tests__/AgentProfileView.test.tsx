import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import AgentProfileView from "@/components/AgentProfileView";
import { I18nProvider } from "@/context/I18nContext";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
}));

const agent = {
  id: "agent-a",
  name: "OOMU",
  description: "Keeps work moving.",
  systemPrompt: "Keep work moving.",
  endpoint: {
    provider: "local_model",
    modelId: "gemma-4-2b",
  },
};

const templateOptions = [
  {
    id: "everyday_agent",
    name: "Everyday Agent",
    description: "Balanced helper.",
    instructions: "Help clearly.",
    attributes: ["friendly"],
    origin: "system" as const,
  },
];

const installedMods = [
  {
    id: "ai.eldris.mods.alignment",
    name: "Core Alignment Matrix",
    description: "Controls alignment-specific behavior.",
    isActive: true,
    agentConfigSchema: {
      title: "Core Alignment Matrix",
      properties: {
        alignment: {
          type: "string",
          title: "Alignment",
          default: "True Neutral",
          "ui:widget": "grid-3x3",
          enum: [
            "Creative Partner",
            "Lawful Good",
            "Neutral Good",
            "Chaotic Good",
            "Lawful Neutral",
            "True Neutral",
            "Chaotic Neutral",
            "Lawful Evil",
            "Neutral Evil",
            "Chaotic Evil",
          ],
        },
      },
    },
  },
  {
    id: "ai.eldris.mods.pundamentals",
    name: "Pundamentals",
    description: "Adds context-aware puns.",
    isActive: true,
  },
  {
    id: "ai.eldris.mods.briefing-coach",
    name: "Briefing Coach",
    description: "Adds concise briefing structure.",
    isActive: true,
  },
  {
    id: "ai.eldris.mods.inactive",
    name: "Inactive Mod",
    description: "Should not be assignable.",
    isActive: false,
  },
];

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((command: string) => {
    if (command === "hydrate_agent_prompt_context") {
      return Promise.resolve(null);
    }
    if (command === "list_installed_mods") {
      return Promise.resolve(installedMods);
    }
    if (command === "get_agent_mods") {
      return Promise.resolve(["ai.eldris.mods.pundamentals"]);
    }
    if (command === "bind_mod_to_agent" || command === "unbind_mod_to_agent") {
      return Promise.resolve(null);
    }
    return Promise.resolve(null);
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("AgentProfileView mod bindings", () => {
  it("updates the agent model behavior maximum output token limit", async () => {
    const onUpdate = vi.fn();

    render(
      <AgentProfileView
        agent={agent}
        configuredProviders={[]}
        onBack={vi.fn()}
        onDelete={vi.fn()}
        onToggleArchive={vi.fn()}
        onUpdate={onUpdate}
        templateOptions={templateOptions}
      />,
      { wrapper: I18nProvider },
    );

    const slider = await screen.findByLabelText("Maximum output tokens");

    expect(slider).toHaveAttribute("min", "1024");
    expect(slider).toHaveAttribute("max", "8192");
    expect(slider).toHaveAttribute("step", "1024");
    expect(slider).toHaveValue("2048");

    fireEvent.change(slider, { target: { value: "8192" } });

    await waitFor(() => {
      const updatedAgent = onUpdate.mock.calls.at(-1)?.[0];
      expect(updatedAgent?.personalityProfile?.modelBehavior.maxOutputTokens).toBe(8192);
    });
  });

  it("renders globally enabled mods and binds or unbinds them for the selected agent", async () => {
    const user = userEvent.setup();
    const onModBindingsChange = vi.fn();

    render(
      <AgentProfileView
        agent={agent}
        configuredProviders={[]}
        onBack={vi.fn()}
        onDelete={vi.fn()}
        onModBindingsChange={onModBindingsChange}
        onToggleArchive={vi.fn()}
        onUpdate={vi.fn()}
        templateOptions={templateOptions}
      />,
      { wrapper: I18nProvider },
    );

    expect(await screen.findByText("Active capability mods")).toBeInTheDocument();
    const pundamentals = await screen.findByRole("checkbox", { name: /Pundamentals/ });
    const briefingCoach = await screen.findByRole("checkbox", { name: /Briefing Coach/ });

    expect(pundamentals).toBeChecked();
    expect(briefingCoach).not.toBeChecked();
    expect(screen.queryByText("Inactive Mod")).not.toBeInTheDocument();

    await user.click(pundamentals);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("unbind_mod_to_agent", {
        agentId: "agent-a",
        modId: "ai.eldris.mods.pundamentals",
      });
    });
    expect(onModBindingsChange).toHaveBeenCalledWith("agent-a", []);

    await user.click(briefingCoach);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("bind_mod_to_agent", {
        agentId: "agent-a",
        modId: "ai.eldris.mods.briefing-coach",
      });
    });
    expect(onModBindingsChange).toHaveBeenCalledWith("agent-a", [
      {
        id: "ai.eldris.mods.briefing-coach",
        name: "Briefing Coach",
      },
    ]);
  });

});

describe("AgentProfileView schema-driven mod settings", () => {

  it("updates the nested alignment mod configuration on the agent profile", async () => {
    const user = userEvent.setup();
    const onUpdate = vi.fn();
    invokeMock.mockImplementation((command: string) => {
      if (command === "hydrate_agent_prompt_context") {
        return Promise.resolve(null);
      }
      if (command === "list_installed_mods") {
        return Promise.resolve(installedMods);
      }
      if (command === "get_agent_mods") {
        return Promise.resolve(["ai.eldris.mods.alignment"]);
      }
      return Promise.resolve(null);
    });

    render(
      <AgentProfileView
        agent={{
          ...agent,
          personalityProfile: {
            schemaVersion: 1,
            identity: {
              displayName: "OOMU",
              role: "Operator",
            },
            personality: {
              summary: "Keeps work moving.",
              traits: ["steady"],
              tone: "Focused.",
            },
            relationship: {
              userAddress: "the user",
              boundaries: ["Stay grounded."],
            },
            modelBehavior: {
              baseModelDisclosure: "runtime_only",
              nameQuestionBehavior: "agent_name",
            },
            mod_configurations: {
              "ai.eldris.mods.alignment": {
                alignment: "Lawful Good",
                note: "preserve me",
              },
            },
          },
        }}
        configuredProviders={[]}
        onBack={vi.fn()}
        onDelete={vi.fn()}
        onToggleArchive={vi.fn()}
        onUpdate={onUpdate}
        templateOptions={templateOptions}
      />,
      { wrapper: I18nProvider },
    );

    const configureButton = await screen.findByRole("button", {
      name: /Configure Core Alignment Matrix/,
    });
    expect(configureButton).toBeInTheDocument();
    await user.click(configureButton);
    expect(screen.queryByRole("combobox", { name: "Alignment" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Chaotic Good" }));

    await waitFor(() => {
      const updatedAgent = onUpdate.mock.calls.at(-1)?.[0];
      expect(updatedAgent?.personalityProfile?.mod_configurations?.["ai.eldris.mods.alignment"]).toEqual({
        alignment: "Chaotic Good",
        note: "preserve me",
      });
    });
  });

  it("keeps only one schema-driven mod configuration panel open", async () => {
    const user = userEvent.setup();
    const configurableMods = [
      {
        id: "ai.eldris.mods.risk-guardrails",
        name: "Risk Guardrails",
        description: "Controls risk thresholds.",
        isActive: true,
        agentConfigSchema: {
          title: "Risk Guardrails",
          properties: {
            riskLimit: {
              type: "number",
              title: "Risk limit",
              minimum: 0,
              maximum: 1,
              default: 0.2,
            },
          },
        },
      },
      {
        id: "ai.eldris.mods.briefing-coach",
        name: "Briefing Coach",
        description: "Adds concise briefing structure.",
        isActive: true,
        agentConfigSchema: {
          title: "Briefing Coach Settings",
          properties: {
            mode: {
              type: "string",
              title: "Mode",
              default: "Concise",
              enum: ["Concise", "Detailed"],
            },
          },
        },
      },
    ];
    invokeMock.mockImplementation((command: string) => {
      if (command === "hydrate_agent_prompt_context") {
        return Promise.resolve(null);
      }
      if (command === "list_installed_mods") {
        return Promise.resolve(configurableMods);
      }
      if (command === "get_agent_mods") {
        return Promise.resolve([
          "ai.eldris.mods.risk-guardrails",
          "ai.eldris.mods.briefing-coach",
        ]);
      }
      return Promise.resolve(null);
    });

    render(
      <AgentProfileView
        agent={agent}
        configuredProviders={[]}
        onBack={vi.fn()}
        onDelete={vi.fn()}
        onToggleArchive={vi.fn()}
        onUpdate={vi.fn()}
        templateOptions={templateOptions}
      />,
      { wrapper: I18nProvider },
    );

    await user.click(await screen.findByRole("button", { name: /Configure Risk Guardrails/ }));
    expect(screen.getByLabelText("Risk limit")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Configure Briefing Coach/ }));
    expect(screen.queryByLabelText("Risk limit")).not.toBeInTheDocument();
    expect(screen.getByText("Briefing Coach Settings")).toBeInTheDocument();
    expect(screen.getByLabelText("Mode")).toBeInTheDocument();
  });
});
