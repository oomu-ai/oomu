export type ChannelPlatform = "telegram" | "discord" | "slack";

type ChannelConnectionState =
  | "linked"
  | "unlinked"
  | "configured"
  | "active"
  | "inactive"
  | "unsupported"
  | "error";

export type ChannelStatus = {
  platform: ChannelPlatform;
  label: string;
  isActive: boolean;
  connectionState: ChannelConnectionState;
  ownerId: string | null;
  allowedChannelIds?: string[];
  workerState: string;
  lastCheckedAtMs: number | null;
  detail: string | null;
};

export function isChannelReady(status: ChannelStatus) {
  if (
    status.connectionState === "error" ||
    status.connectionState === "unsupported"
  ) {
    return false;
  }
  return (
    status.workerState === "running" &&
    ["linked", "configured", "active"].includes(status.connectionState)
  );
}

export function hasChannelError(status: ChannelStatus) {
  return (
    status.connectionState === "error" ||
    ["degraded", "stopped", "artifact_invalid"].includes(status.workerState)
  );
}
