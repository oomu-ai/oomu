"use client";

import { useEffect, useState } from "react";
import { invoke } from "@/lib/invoke";

export const FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY =
  "oomu.chat.firstRunWelcome.dismissed.v1";

export type DecisionBriefCompletionState =
  | "checking"
  | "complete"
  | "incomplete"
  | "unavailable";

const subscribers = new Set<() => void>();

export function firstRunChatWelcomeIsDismissed() {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY) === "1";
  } catch {
    return false;
  }
}

export function dismissFirstRunChatWelcome() {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY, "1");
  } catch {
    // In-memory subscribers still update the current view when storage is unavailable.
  }
  subscribers.forEach((subscriber) => subscriber());
}

export function subscribeToFirstRunChatWelcome(subscriber: () => void) {
  const handleStorage = (event: StorageEvent) => {
    if (event.key === FIRST_RUN_CHAT_WELCOME_DISMISSED_KEY) subscriber();
  };
  subscribers.add(subscriber);
  window.addEventListener("storage", handleStorage);
  return () => {
    subscribers.delete(subscriber);
    window.removeEventListener("storage", handleStorage);
  };
}

export function useDecisionBriefCompletion(): DecisionBriefCompletionState {
  const [state, setState] = useState<DecisionBriefCompletionState>("checking");

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const projects = await invoke<Array<{ projectId: string }>>("list_projects");
        if (!active) return;
        if (projects.length === 0) {
          setState("incomplete");
          return;
        }
        const results = await Promise.allSettled(
          projects.map((project) =>
            invoke<{ readyOnDemand: boolean; readyWeekly: boolean }>(
              "get_weekly_decision_brief_status",
              { request: { projectId: project.projectId } },
            ),
          ),
        );
        if (!active) return;
        const complete = results.some(
          (result) =>
            result.status === "fulfilled" &&
            (result.value.readyOnDemand || result.value.readyWeekly),
        );
        if (complete) {
          dismissFirstRunChatWelcome();
          setState("complete");
        } else {
          setState(
            results.every((result) => result.status === "fulfilled")
              ? "incomplete"
              : "unavailable",
          );
        }
      } catch {
        if (active) setState("unavailable");
      }
    })();
    return () => {
      active = false;
    };
  }, []);

  return state;
}
