import { within } from "@testing-library/react";
import { expect } from "vitest";

type InvokeArgs = { request?: Record<string, unknown> };

export function createPlanPersistenceMock(sessionId: string) {
  let assistantText = "";
  return {
    listMessages: () => assistantText
      ? [{
          id: 99,
          sessionId,
          role: "assistant" as const,
          content: assistantText,
          createdAtMs: 99,
        }]
      : [],
    record: (args?: InvokeArgs) => {
      assistantText = String(args?.request?.assistant_text ?? "");
      return { text: assistantText, session_id: sessionId };
    },
  };
}

export function expectReadablePlanPreview(container: HTMLElement, stepCount: number) {
  const preview = container.querySelector<HTMLElement>("[data-oomu-plan-preview='true']");
  expect(preview).not.toBeNull();
  const header = preview?.querySelector<HTMLElement>(":scope > div > div");
  expect(header).toHaveClass("min-w-0");
  expect(header?.querySelector("h3")).toHaveClass("break-words");
  expect(header?.querySelector("p.mt-2")).toHaveClass("break-words");
  for (const item of preview?.querySelectorAll("ol > li") ?? []) {
    expect(item).toHaveClass("break-words");
  }
  expect(within(preview!).getAllByText(/Complete task/)).toHaveLength(stepCount);
  expect(within(preview!).queryByText(/registered task tool/i)).toBeNull();
}
