import { useEffect, useRef } from "react";
import type { RoutineDraft } from "./routineDraft";

export function useRoutineDraftHandoff(
  draft: RoutineDraft | null,
  consume: (draft: RoutineDraft | null) => void,
  begin: (draft: RoutineDraft) => void,
) {
  const consumedDraftIdRef = useRef<string | null>(null);
  useEffect(() => {
    if (!draft) {
      consumedDraftIdRef.current = null;
      return;
    }
    if (consumedDraftIdRef.current === draft.id) return;
    consumedDraftIdRef.current = draft.id;
    begin(draft);
    consume(null);
  }, [begin, consume, draft]);
}
