const SIGNAL_KEY = "oomu.macos-permission-refresh";
const SIGNAL_MAX_AGE_MS = 5 * 60 * 1_000;

type PermissionRefreshSignal = {
  capabilityId: string;
  createdAtMs: number;
};

export function publishMacPermissionRefresh(capabilityId: string) {
  const signal: PermissionRefreshSignal = { capabilityId, createdAtMs: Date.now() };
  try {
    window.sessionStorage.setItem(SIGNAL_KEY, JSON.stringify(signal));
  } catch {
    // The in-window event remains enough when browser storage is unavailable.
  }
  window.dispatchEvent(new CustomEvent("oomu:macos-permissions-refreshed", {
    detail: { capabilityId },
  }));
}

export function consumeMacPermissionRefresh(capabilityId: string) {
  try {
    const raw = window.sessionStorage.getItem(SIGNAL_KEY);
    if (!raw) return false;
    const signal = JSON.parse(raw) as PermissionRefreshSignal;
    if (Date.now() - signal.createdAtMs > SIGNAL_MAX_AGE_MS) {
      window.sessionStorage.removeItem(SIGNAL_KEY);
      return false;
    }
    if (signal.capabilityId !== capabilityId) {
      return false;
    }
    window.sessionStorage.removeItem(SIGNAL_KEY);
    return true;
  } catch {
    return false;
  }
}
