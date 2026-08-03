import { useCallback, useEffect, useState } from "react";
import { routineApi, type RoutineProposal } from "./routineClient";

type RoutineTranslate = (key: string) => string;

export function useRoutineProposal(
  creating: boolean,
  scheduleText: string,
  timezone: string,
  t: RoutineTranslate,
) {
  const [proposal, setProposal] = useState<RoutineProposal | null>(null);
  const [proposalKey, setProposalKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const activeKey = `${scheduleText}\u0000${timezone}`;
  const current = proposalKey === activeKey ? proposal : null;

  const reset = useCallback(() => {
    setProposal(null);
    setProposalKey("");
    setBusy(false);
    setError("");
  }, []);

  useEffect(() => {
    if (!creating) return;
    if (!scheduleText.trim()) {
      const resetTimer = window.setTimeout(() => {
        setBusy(false);
        setError("");
      }, 0);
      return () => window.clearTimeout(resetTimer);
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      setBusy(true);
      setError("");
      void routineApi
        .propose(scheduleText, timezone)
        .then((nextProposal) => {
          if (cancelled) return;
          setProposal(nextProposal);
          setProposalKey(activeKey);
        })
        .catch(() => {
          if (cancelled) return;
          setProposal(null);
          setProposalKey("");
          setError(t("routines.error_schedule"));
        })
        .finally(() => {
          if (!cancelled) setBusy(false);
        });
    }, 350);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [activeKey, creating, scheduleText, t, timezone]);

  return { current, busy, error, reset };
}
