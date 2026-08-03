import { useCallback, useLayoutEffect, useRef, useState } from "react";

export type SessionScopedStateUpdate<T> = T | ((current: T) => T);

export const NEW_CHAT_SESSION_SCOPE = "__new_chat_session__";

export function chatSessionStateScope(sessionId: string) {
  return sessionId.trim() || NEW_CHAT_SESSION_SCOPE;
}

export function upsertByNumericId<T extends { id: number }>(current: T[], next: T) {
  const existingIndex = current.findIndex((entry) => entry.id === next.id);
  if (existingIndex < 0) return [...current, next];
  return current.map((entry, index) => index === existingIndex ? next : entry);
}

export function useSessionScopedState<T>(sessionId: string, initialValue: T) {
  const initialValueRef = useRef(initialValue);
  const [values, setValues] = useState<Record<string, T>>({});
  const activeScope = chatSessionStateScope(sessionId);
  const hasValue = Object.prototype.hasOwnProperty.call(values, activeScope);
  const value = hasValue ? values[activeScope] : initialValue;

  const updateScope = useCallback((scope: string, update: SessionScopedStateUpdate<T>) => {
    setValues((current) => {
      const currentValue = Object.prototype.hasOwnProperty.call(current, scope)
        ? current[scope]
        : initialValueRef.current;
      const nextValue = typeof update === "function"
        ? (update as (current: T) => T)(currentValue)
        : update;
      if (Object.is(nextValue, currentValue)) {
        return current;
      }
      return { ...current, [scope]: nextValue };
    });
  }, []);

  const setForSession = useCallback((targetSessionId: string, update: SessionScopedStateUpdate<T>) => {
    const cleanedSessionId = targetSessionId.trim();
    if (!cleanedSessionId) {
      return;
    }
    updateScope(cleanedSessionId, update);
  }, [updateScope]);

  const setValue = useCallback((update: SessionScopedStateUpdate<T>) => {
    updateScope(activeScope, update);
  }, [activeScope, updateScope]);

  const clearSession = useCallback((targetSessionId: string) => {
    const cleanedSessionId = targetSessionId.trim();
    if (!cleanedSessionId) {
      return;
    }
    setValues((current) => {
      if (!(cleanedSessionId in current)) {
        return current;
      }
      const next = { ...current };
      delete next[cleanedSessionId];
      return next;
    });
  }, []);

  const valueForSession = useCallback((targetSessionId: string) => {
    const scope = chatSessionStateScope(targetSessionId);
    return Object.prototype.hasOwnProperty.call(values, scope)
      ? values[scope]
      : initialValueRef.current;
  }, [values]);

  return [value, setValue, setForSession, clearSession, hasValue, valueForSession] as const;
}

export function useStableEvent<Args extends unknown[], ReturnValue>(
  callback: (...args: Args) => ReturnValue,
) {
  const callbackRef = useRef(callback);
  // Layout synchronization keeps newly visible controls bound to the closure
  // from the DOM commit that made them actionable.
  useLayoutEffect(() => {
    callbackRef.current = callback;
  }, [callback]);
  return useCallback((...args: Args) => callbackRef.current(...args), []);
}
