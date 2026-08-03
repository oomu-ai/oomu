import type { AutoRouteAttention } from "./AutoRouteAttentionCard";
import type { MacPermissionRecoveryDescriptor } from "./MacPermissionRecoveryCard";

const storageKey = "oomu.chat.turn-recovery.v1";
const schema = "oomu.chat.turn_recovery.v1";
const maximumRecords = 32;
const maximumAgeMs = 7 * 24 * 60 * 60 * 1_000;
const safeIdentity = /^[a-zA-Z0-9][a-zA-Z0-9_.:/-]{0,511}$/;
const safeToken = /^[a-z][a-z0-9_]{0,95}$/;

export type TurnRecoveryIdentity = {
  sessionId: string;
  rootTurnId: string;
  turnId: string;
  generationToken: string;
};

export function turnRecoveryIdentityKey(identity: TurnRecoveryIdentity) {
  return JSON.stringify([
    identity.sessionId,
    identity.rootTurnId,
    identity.turnId,
    identity.generationToken,
  ]);
}

export type PersistedAutoRouteRecovery = TurnRecoveryIdentity & {
  type: "auto_route";
  attention: AutoRouteAttention;
  updatedAtMs: number;
};

export type PersistedApplePermissionRecovery = TurnRecoveryIdentity & {
  type: "apple_permission";
  boundary: string;
  code: string;
  descriptor: MacPermissionRecoveryDescriptor;
  updatedAtMs: number;
};

export type PersistedTurnRecovery =
  | PersistedAutoRouteRecovery
  | PersistedApplePermissionRecovery;

type StoredEnvelope = {
  schema: typeof schema;
  records: PersistedTurnRecovery[];
};

function defaultStorage() {
  return typeof window === "undefined" ? null : window.localStorage;
}

function validIdentity(value: unknown): value is string {
  return typeof value === "string" && safeIdentity.test(value.trim());
}

function validTimestamp(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0;
}

function validAttention(value: unknown, identity: TurnRecoveryIdentity): value is AutoRouteAttention {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const attention = value as Partial<AutoRouteAttention>;
  return attention.sessionId === identity.sessionId
    && attention.rootTurnId === identity.rootTurnId
    && attention.turnId === identity.turnId
    && attention.generationToken === identity.generationToken
    && (attention.localProviderId === "" || validIdentity(attention.localProviderId))
    && (attention.localModelId === "" || validIdentity(attention.localModelId))
    && (attention.recommendedLocalProviderId === "" || validIdentity(attention.recommendedLocalProviderId))
    && (attention.recommendedLocalModelId === "" || validIdentity(attention.recommendedLocalModelId))
    && (attention.cloudModelId === "" || validIdentity(attention.cloudModelId))
    && typeof attention.failureCode === "string"
    && safeToken.test(attention.failureCode)
    && (attention.failureBoundary === null
      || (typeof attention.failureBoundary === "string" && safeToken.test(attention.failureBoundary)))
    && ["choose_model", "preparing", "timeout", "cloud_setup", "saved_work_check", "interrupted", "unknown"]
      .includes(attention.kind ?? "");
}

function parseRecord(value: unknown): PersistedTurnRecovery | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const record = value as Partial<PersistedTurnRecovery>;
  const updatedAtMs = record.updatedAtMs;
  if (
    !validIdentity(record.sessionId)
    || !validIdentity(record.turnId)
    || !validIdentity(record.generationToken)
    || !validTimestamp(updatedAtMs)
    || Date.now() - updatedAtMs > maximumAgeMs
  ) {
    return null;
  }
  const rootTurnId = validIdentity(record.rootTurnId) ? record.rootTurnId : record.turnId;
  const identity = {
    sessionId: record.sessionId,
    rootTurnId,
    turnId: record.turnId,
    generationToken: record.generationToken,
  };
  if (record.type === "auto_route") {
    if (!record.attention) return null;
    const normalizedAttention = {
      ...record.attention,
      rootTurnId: record.attention.rootTurnId ?? rootTurnId,
    };
    return validAttention(normalizedAttention, identity)
      ? { ...record, rootTurnId, attention: normalizedAttention } as PersistedAutoRouteRecovery
      : null;
  }
  if (record.type !== "apple_permission") return null;
  const permission = record as Partial<PersistedApplePermissionRecovery>;
  const descriptor = permission.descriptor;
  if (
    typeof permission.code !== "string"
    || !safeToken.test(permission.code)
    || typeof permission.boundary !== "string"
    || !safeToken.test(permission.boundary)
    || !descriptor
    || !validIdentity(descriptor.capabilityId)
    || !["not_requested", "denied", "limited", "restricted", "stale", "timeout", "unsupported"]
      .includes(descriptor.state)
  ) {
    return null;
  }
  return { ...record, rootTurnId } as PersistedApplePermissionRecovery;
}

export function readTurnRecoveries(storage: Storage | null = defaultStorage()) {
  if (!storage) return [];
  try {
    const parsed = JSON.parse(storage.getItem(storageKey) ?? "null") as Partial<StoredEnvelope> | null;
    if (parsed?.schema !== schema || !Array.isArray(parsed.records)) return [];
    return parsed.records.flatMap((record) => {
      const valid = parseRecord(record);
      return valid ? [valid] : [];
    }).slice(-maximumRecords);
  } catch {
    return [];
  }
}

function writeTurnRecoveries(records: PersistedTurnRecovery[], storage: Storage | null) {
  if (!storage) return false;
  try {
    const envelope: StoredEnvelope = { schema, records: records.slice(-maximumRecords) };
    storage.setItem(storageKey, JSON.stringify(envelope));
    return true;
  } catch {
    return false;
  }
}

export function persistTurnRecovery(
  record: PersistedTurnRecovery,
  storage: Storage | null = defaultStorage(),
) {
  const valid = parseRecord(record);
  if (!valid) return false;
  const others = readTurnRecoveries(storage).filter((candidate) =>
    candidate.type !== valid.type
    || turnRecoveryIdentityKey(candidate) !== turnRecoveryIdentityKey(valid)
  );
  return writeTurnRecoveries([...others, valid], storage);
}

export function readTurnRecovery<T extends PersistedTurnRecovery["type"]>(
  sessionId: string,
  type: T,
  storage: Storage | null = defaultStorage(),
) {
  const record = [...readTurnRecoveries(storage)].reverse().find((candidate) =>
    candidate.sessionId === sessionId && candidate.type === type
  );
  return (record ?? null) as Extract<PersistedTurnRecovery, { type: T }> | null;
}

export function readMatchingTurnRecovery<T extends PersistedTurnRecovery["type"]>(
  sessionId: string,
  type: T,
  recoverableIdentityKeys: ReadonlySet<string>,
  storage: Storage | null = defaultStorage(),
) {
  const record = [...readTurnRecoveries(storage)].reverse().find((candidate) =>
    candidate.sessionId === sessionId
    && candidate.type === type
    && recoverableIdentityKeys.has(turnRecoveryIdentityKey(candidate))
  );
  return (record ?? null) as Extract<PersistedTurnRecovery, { type: T }> | null;
}

export function clearTurnRecovery(
  identity: TurnRecoveryIdentity,
  type: PersistedTurnRecovery["type"],
  storage: Storage | null = defaultStorage(),
) {
  const current = readTurnRecoveries(storage);
  const next = current.filter((record) => !(
    record.type === type
    && record.sessionId === identity.sessionId
    && record.rootTurnId === identity.rootTurnId
    && record.turnId === identity.turnId
    && record.generationToken === identity.generationToken
  ));
  return next.length === current.length || writeTurnRecoveries(next, storage);
}

export function clearTerminalTurnRecoveries(
  sessionId: string,
  terminalTurnIdentityKeys: ReadonlySet<string>,
  storage: Storage | null = defaultStorage(),
) {
  if (!sessionId || terminalTurnIdentityKeys.size === 0) return true;
  const current = readTurnRecoveries(storage);
  const next = current.filter((record) =>
    record.sessionId !== sessionId
    || !terminalTurnIdentityKeys.has(turnRecoveryIdentityKey(record))
  );
  return next.length === current.length || writeTurnRecoveries(next, storage);
}
