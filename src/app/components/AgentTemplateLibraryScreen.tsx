"use client";

import type { FormEvent } from "react";
import { useI18n } from "@/context/I18nContext";
import { ScreenHeader } from "./HomeChrome";
import {
  attributeLabel,
  buildTemplatePreview,
  getCapabilitiesForTemplate,
  instructionAttributeOptions,
  type AgentInstructionTemplate,
} from "../homeAgents";

type AgentTemplateLibraryScreenProps = {
  activeAgentTemplate: AgentInstructionTemplate;
  agentTemplateOptions: AgentInstructionTemplate[];
  aiInstructionsProgress: string;
  canSaveCustomTemplate: boolean;
  customTemplateAttributes: string[];
  customTemplateDescription: string;
  customTemplateInstructions: string;
  customTemplateName: string;
  isCreatingTemplate: boolean;
  isGeneratingAIInstructions: boolean;
  onActiveTemplateChange: (templateId: string) => void;
  onBackToAgents: () => void;
  onCustomTemplateAttributeToggle: (attributeId: string) => void;
  onCustomTemplateDescriptionChange: (description: string) => void;
  onCustomTemplateInstructionsChange: (instructions: string) => void;
  onCustomTemplateNameChange: (name: string) => void;
  onDeleteTemplate: (templateId: string) => void;
  onGenerateInstructions: () => void;
  onResetCustomTemplate: () => void;
  onSaveCustomTemplate: (event: FormEvent<HTMLFormElement>) => void;
  onSetCreatingTemplate: (isCreating: boolean) => void;
  onShowRawPromptChange: (showRawPrompt: boolean) => void;
  onUseTemplate: (templateId: string) => void;
  showRawPrompt: boolean;
};

export function AgentTemplateLibraryScreen({
  activeAgentTemplate,
  agentTemplateOptions,
  aiInstructionsProgress,
  canSaveCustomTemplate,
  customTemplateAttributes,
  customTemplateDescription,
  customTemplateInstructions,
  customTemplateName,
  isCreatingTemplate,
  isGeneratingAIInstructions,
  onActiveTemplateChange,
  onBackToAgents,
  onCustomTemplateAttributeToggle,
  onCustomTemplateDescriptionChange,
  onCustomTemplateInstructionsChange,
  onCustomTemplateNameChange,
  onDeleteTemplate,
  onGenerateInstructions,
  onResetCustomTemplate,
  onSaveCustomTemplate,
  onSetCreatingTemplate,
  onShowRawPromptChange,
  onUseTemplate,
  showRawPrompt,
}: AgentTemplateLibraryScreenProps) {
  const { t } = useI18n();
  const draftTemplate: AgentInstructionTemplate = {
    id: "draft",
    name: customTemplateName.trim() || t("agents.template_screen.draft_name"),
    description:
      customTemplateDescription.trim() || t("agents.template_screen.draft_description"),
    instructions:
      customTemplateInstructions.trim() ||
      t("agents.template_screen.draft_instructions"),
    attributes: customTemplateAttributes,
    origin: "custom",
  };
  const previewTemplate = isCreatingTemplate ? draftTemplate : activeAgentTemplate;
  const capabilities = getCapabilitiesForTemplate(previewTemplate);

  return (
    <section className="flex min-h-0 flex-1 flex-col gap-4 overflow-hidden">
      <ScreenHeader title={t("agents.template_screen.title")} showBorder>
        {isCreatingTemplate ? (
          <button
            className="w-full rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] lg:w-auto"
            onClick={() => onSetCreatingTemplate(false)}
            type="button"
          >
            {t("agents.template_screen.back_to_library")}
          </button>
        ) : (
          <>
            <button
              className="w-full rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] lg:w-auto"
              onClick={onBackToAgents}
              type="button"
            >
              {t("agents.template_screen.back")}
            </button>
            <button
              className="w-full rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-transparent px-3 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] lg:w-auto"
              onClick={() => onSetCreatingTemplate(true)}
              type="button"
            >
              {t("agents.template_screen.new_template")}
            </button>
            <button
              className="w-full rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--inverse-background)] px-3 py-2 text-sm font-medium text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] lg:w-auto"
              onClick={() => onUseTemplate(activeAgentTemplate.id)}
              type="button"
            >
              {t("agents.template_screen.use_template")}
            </button>
          </>
        )}
      </ScreenHeader>

      <div className="grid min-h-0 flex-1 gap-4 overflow-hidden xl:grid-cols-[minmax(18rem,2fr)_minmax(0,3fr)]">
        {isCreatingTemplate ? (
          <form className="flex min-w-0 flex-col gap-3 overflow-y-auto pr-1" onSubmit={onSaveCustomTemplate}>
            <div>
              <h2 className="text-sm font-semibold text-[var(--foreground)]">
                {t("agents.template_screen.create_title")}
              </h2>
              <p className="mt-1 text-sm leading-6 text-[var(--foreground-muted)]">
                {t("agents.template_screen.create_description")}
              </p>
            </div>

            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <label className="flex flex-col gap-1">
                <span className="text-xs font-medium text-[var(--foreground-muted)]">
                  {t("agents.template_screen.name")}
                </span>
                <input
                  className="border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm text-[var(--foreground)] outline-none transition-colors focus:bg-[var(--accent-background)]"
                  onChange={(event) => onCustomTemplateNameChange(event.target.value)}
                  placeholder={t("agents.template_screen.name_placeholder")}
                  required
                  value={customTemplateName}
                />
              </label>

              <label className="flex flex-col gap-1">
                <span className="text-xs font-medium text-[var(--foreground-muted)]">
                  {t("agents.template_screen.description")}
                </span>
                <input
                  className="border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-medium tracking-tight text-[var(--foreground)] outline-none transition-colors focus:bg-[var(--accent-background)]"
                  onChange={(event) => onCustomTemplateDescriptionChange(event.target.value)}
                  placeholder={t("agents.template_screen.description_placeholder")}
                  value={customTemplateDescription}
                />
              </label>
            </div>

            <div className="flex flex-col gap-1">
              <label className="flex flex-col gap-1">
                <span className="text-xs font-medium text-[var(--foreground-muted)]">
                  {t("agents.template_screen.instructions")}
                </span>
                <textarea
                  className="min-h-[6rem] resize-y border border-[var(--border-strong)] bg-[var(--accent-background)] px-3 py-2 text-sm font-medium leading-6 text-[var(--foreground)] outline-none transition-colors focus:bg-[var(--background)]"
                  onChange={(event) => onCustomTemplateInstructionsChange(event.target.value)}
                  placeholder={t("agents.template_screen.instructions_placeholder")}
                  required
                  value={customTemplateInstructions}
                />
              </label>
              <div className="mt-1 flex items-center justify-between gap-3">
                <button
                  className="flex items-center gap-1.5 border border-[var(--border-strong)] bg-[var(--background)] px-3 py-1.5 text-xs font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={!customTemplateName.trim() || isGeneratingAIInstructions}
                  onClick={onGenerateInstructions}
                  type="button"
                >
                  <svg className="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d="M13 10V3L4 14h7v7l9-11h-7z" />
                  </svg>
                  {isGeneratingAIInstructions
                    ? t("agents.template_screen.drafting_ai")
                    : t("agents.template_screen.create_with_ai")}
                </button>
                {aiInstructionsProgress && (
                  <span className="text-xs text-[var(--foreground-muted)]">
                    {aiInstructionsProgress}
                  </span>
                )}
              </div>
            </div>

            <div className="flex flex-col gap-1.5">
              <span className="text-xs font-medium text-[var(--foreground-muted)]">
                {t("agents.template_screen.behavior")}
              </span>
              <div className="flex flex-wrap gap-1.5">
                {instructionAttributeOptions.map((attribute) => {
                  const selected = customTemplateAttributes.includes(attribute.id);

                  return (
                    <button
                      className={`rounded-[var(--radius-sm)] border px-2.5 py-1 text-xs font-medium transition-colors ${
                        selected
                          ? "border-[var(--border-strong)] bg-[var(--inverse-background)] text-[var(--inverse-foreground)]"
                          : "border-[var(--border-strong)] bg-[var(--background)] text-[var(--foreground)] hover:bg-[var(--accent-background)]"
                      }`}
                      key={attribute.id}
                      onClick={() => onCustomTemplateAttributeToggle(attribute.id)}
                      type="button"
                    >
                      {attribute.label}
                    </button>
                  );
                })}
              </div>
            </div>

            <div className="mt-2 flex flex-col gap-2 sm:flex-row">
              <button
                className="flex-1 rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--inverse-background)] px-3 py-2 text-sm font-medium text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)] disabled:cursor-not-allowed disabled:opacity-50"
                disabled={!canSaveCustomTemplate}
                type="submit"
              >
                {t("agents.template_screen.save_template")}
              </button>
              <button
                className="border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
                onClick={onResetCustomTemplate}
                type="button"
              >
                {t("common.reset")}
              </button>
            </div>
          </form>
        ) : (
          <section className="flex min-w-0 flex-col gap-3 overflow-y-auto pr-1">
            <div className="flex items-center justify-between gap-3">
              <h2 className="text-sm font-semibold text-[var(--foreground)]">
                {t("agents.template_screen.library")}
              </h2>
              <span className="text-xs text-[var(--foreground-subtle)]">
                {t("agents.template_screen.templates_count", {
                  count: agentTemplateOptions.length,
                })}
              </span>
            </div>

            <div className="grid gap-3">
              {agentTemplateOptions.map((template) => {
                const isActive = activeAgentTemplate.id === template.id;

                return (
                  <article
                    className={`rounded-[var(--radius-md)] border p-4 transition-colors ${
                      isActive
                        ? "border-[var(--border-strong)] bg-[var(--inverse-background)] text-[var(--inverse-foreground)]"
                        : "border-[var(--border-strong)] bg-[var(--background)] text-[var(--foreground)] hover:bg-[var(--accent-background)]"
                    }`}
                    key={template.id}
                  >
                    <button
                      className="block w-full text-left"
                      onClick={() => onActiveTemplateChange(template.id)}
                      type="button"
                    >
                      <div className="flex items-start justify-between gap-3">
                        <div>
                          <p className="text-base font-semibold tracking-tight">
                            {template.name}
                          </p>
                          <p
                            className={`mt-1 text-xs ${
                              isActive ? "text-[var(--inverse-foreground)] opacity-70" : "text-[var(--foreground-subtle)]"
                            }`}
                          >
                            {template.origin === "custom"
                              ? t("agents.template_screen.custom")
                              : t("agents.template_screen.built_in")}
                          </p>
                        </div>
                        <span
                          className={`text-xs ${
                            isActive ? "text-[var(--inverse-foreground)] opacity-70" : "text-[var(--foreground-subtle)]"
                          }`}
                        >
                          {t("agents.template_screen.traits_count", {
                            count: template.attributes.length,
                          })}
                        </span>
                      </div>
                      <p
                        className={`mt-3 text-sm leading-5 ${
                          isActive ? "text-[var(--inverse-foreground)] opacity-80" : "text-[var(--foreground-muted)]"
                        }`}
                      >
                        {template.description}
                      </p>
                      <div className="mt-4 flex flex-wrap gap-2">
                        {template.attributes.map((attributeId) => (
                          <span
                            className={`rounded-[var(--radius-sm)] border px-2 py-1 text-[11px] ${
                              isActive
                                ? "border-[var(--inverse-foreground)] text-[var(--inverse-foreground)] opacity-80"
                                : "border-[var(--border-soft)] text-[var(--foreground-muted)]"
                            }`}
                            key={attributeId}
                          >
                            {attributeLabel(attributeId)}
                          </span>
                        ))}
                      </div>
                    </button>

                    {template.origin === "custom" && (
                      <button
                        className={`mt-4 rounded-[var(--radius-sm)] border px-3 py-2 text-sm font-medium transition-colors ${
                          isActive
                            ? "border-[var(--inverse-foreground)] text-[var(--inverse-foreground)] hover:bg-[var(--inverse-foreground)] hover:text-[var(--inverse-background)]"
                            : "border-[var(--border-strong)] text-[var(--foreground)] hover:bg-[var(--fill-hover)]"
                        }`}
                        onClick={() => onDeleteTemplate(template.id)}
                        type="button"
                      >
                        {t("agents.template_screen.delete_template")}
                      </button>
                    )}
                  </article>
                );
              })}
            </div>
          </section>
        )}

        <section className="flex h-full min-w-0 flex-col gap-3 overflow-hidden">
          <h2 className="text-sm font-semibold text-[var(--foreground)]">
            {t("agents.template_screen.preview")}
          </h2>
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--accent-background)] p-4">
            <div className="shrink-0 rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-4">
              <p className="text-xs text-[var(--foreground-muted)]">
                {isCreatingTemplate
                  ? t("agents.template_screen.draft")
                  : t("agents.template_screen.selected_template")}
              </p>
              <h3 className="mt-2 text-2xl font-semibold tracking-tight">
                {previewTemplate.name}
              </h3>
              <p className="mt-2 text-sm leading-6 text-[var(--foreground-muted)]">
                {previewTemplate.description}
              </p>
            </div>
            <button
              className="mt-4 flex w-full shrink-0 items-center justify-between rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-4 py-2.5 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
              onClick={() => onShowRawPromptChange(!showRawPrompt)}
              type="button"
            >
              <span>
                {showRawPrompt
                  ? t("agents.template_screen.hide_system_prompt")
                  : t("agents.template_screen.show_system_prompt")}
              </span>
              <span className="text-[10px] text-[var(--foreground-muted)]">
                {showRawPrompt ? "▲" : "▼"}
              </span>
            </button>

            {showRawPrompt ? (
              <pre className="mt-4 min-h-0 flex-1 overflow-y-auto whitespace-pre-wrap rounded-[var(--radius-md)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4 font-mono text-xs leading-6 text-[var(--foreground)]">
                {buildTemplatePreview(previewTemplate)}
              </pre>
            ) : (
              <div className="mt-4 flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)] p-5 text-left">
                <div className="flex flex-col gap-2">
                  <h4 className="border-b border-[var(--border-soft)] pb-1.5 text-sm font-semibold text-[var(--foreground)]">
                    {t("agents.template_screen.help_title")}
                  </h4>
                  <ul className="flex flex-col gap-3">
                    {capabilities.characteristics.map((char, index) => {
                      const colonIndex = char.indexOf(":");
                      const label = colonIndex > -1 ? char.slice(0, colonIndex) : "";
                      const text = colonIndex > -1 ? char.slice(colonIndex + 1) : char;

                      return (
                        <li className="flex items-start gap-2 text-xs leading-5 text-[var(--foreground-muted)]" key={index}>
                          <span className="mt-0.5 text-[10px] text-[var(--foreground-subtle)]">-</span>
                          <div>
                            {label && <strong className="mr-1 font-semibold text-[var(--foreground)]">{label}:</strong>}
                            {text}
                          </div>
                        </li>
                      );
                    })}
                  </ul>
                </div>

                <div className="flex flex-col gap-2">
                  <h4 className="border-b border-[var(--border-soft)] pb-1.5 text-sm font-semibold text-[var(--foreground)]">
                    {t("agents.template_screen.examples_title")}
                  </h4>
                  <div className="flex flex-col gap-2.5">
                    {capabilities.examples.map((example, index) => (
                      <div
                        className="cursor-pointer select-all rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3 text-xs leading-5 text-[var(--foreground-muted)] transition-colors hover:border-[var(--border-strong)] hover:bg-[var(--background)]"
                        key={index}
                      >
                        &ldquo;{example}&rdquo;
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            )}
            <p className="mt-3 shrink-0 text-xs leading-5 text-[var(--foreground-muted)]">
              {t("agents.template_screen.preview_note")}
            </p>
          </div>
        </section>
      </div>
    </section>
  );
}
