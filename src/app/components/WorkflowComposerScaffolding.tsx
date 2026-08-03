"use client";

import type { WorkflowIrTemplateExample } from "./workflowLibrary";

type TranslateFn = (
  key: string,
  variables?: Record<string, string | number>,
) => string;

type WorkflowTemplate = WorkflowIrTemplateExample;

export function workflowTemplateName(template: WorkflowTemplate, t: TranslateFn) {
  const key = `workflows.templates.${template.id}.name`;
  const translated = t(key);
  return translated === key ? template.name : translated;
}

export function workflowTemplateDescription(
  template: WorkflowTemplate,
  t: TranslateFn,
) {
  const key = `workflows.templates.${template.id}.description`;
  const translated = t(key);
  return translated === key ? template.description : translated;
}
