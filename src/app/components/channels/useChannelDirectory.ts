"use client";

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@/lib/invoke";
import {
  integrationApi,
  type ConnectorAccount,
  type ConnectorManifest,
} from "../integrations/integrationClient";
import type { ChannelStatus } from "../routines/channelReadiness";

export function useChannelDirectory() {
  const [statuses, setStatuses] = useState<ChannelStatus[]>([]);
  const [accounts, setAccounts] = useState<ConnectorAccount[]>([]);
  const [manifests, setManifests] = useState<ConnectorManifest[]>([]);
  const [loadFailed, setLoadFailed] = useState(false);

  const load = useCallback(async () => {
    try {
      const [nextStatuses, nextAccounts, nextManifests] = await Promise.all([
        invoke<ChannelStatus[]>("get_channel_statuses"),
        integrationApi.accounts(),
        integrationApi.manifests(),
      ]);
      setStatuses(nextStatuses);
      setAccounts(nextAccounts);
      setManifests(nextManifests);
      setLoadFailed(false);
      return { accounts: nextAccounts, statuses: nextStatuses };
    } catch {
      setLoadFailed(true);
      return null;
    }
  }, []);

  useEffect(() => {
    const firstLoad = window.setTimeout(() => void load(), 0);
    const refresh = window.setInterval(() => void load(), 4_000);
    return () => {
      window.clearTimeout(firstLoad);
      window.clearInterval(refresh);
    };
  }, [load]);

  return { accounts, load, loadFailed, manifests, statuses };
}
