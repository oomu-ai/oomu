"use client";

import { invoke, isTauriRuntime } from "@/lib/invoke";
import { ApprovalDialogFrame } from "@/components/approvals/ApprovalDialogFrame";
import { useApprovalDialogTurn } from "@/context/ApprovalContext";
import { useI18n } from "@/context/I18nContext";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import {
  ModTrustBadge,
  ModTrustSummary,
  modTrustPresentation,
  type ModIntegrityState,
  type ModReviewState,
} from "./modTrustPresentation";

type ModPermission = {
  label: string;
  detail: string;
};

type InstalledMod = {
  id: string;
  name: string;
  description: string;
  isActive: boolean;
  version: string;
  author: string;
  category: string;
  packageSize: string;
  lastUpdated: string;
  permissions: ModPermission[];
  endpoints: string[];
  reviewState?: ModReviewState;
  publisherIdentityVerified?: boolean;
  integrityState?: ModIntegrityState;
  isBuiltIn?: boolean;
};
type CapabilityGrant={capability:"file"|"network"|"connector"|"model"|"executable"|"schedule"|"child_agent"|"mutation";boundedScope:string;reason:string};
type CapabilityBundle={bundleId:string;packageVersion:string;modId:string;name:string;publisherName:string;publisherIdentityVerified:boolean;reviewState:ModReviewState;integrityState?:ModIntegrityState;compatibilityState:string;capabilities:CapabilityGrant[];projectIds:string[];installState:string;previousVersion:string|null;updatedAtMs:number};
type BundleActivationIntent = "install" | "enable";

const BUNDLE_CAPABILITY_SENTENCE_KEYS: Record<CapabilityGrant["capability"], string> = {
  child_agent: "mods.capability_sentences.child_agent",
  connector: "mods.capability_sentences.connector",
  executable: "mods.capability_sentences.executable",
  file: "mods.capability_sentences.file",
  model: "mods.capability_sentences.model",
  mutation: "mods.capability_sentences.mutation",
  network: "mods.capability_sentences.network",
  schedule: "mods.capability_sentences.schedule",
};

type ModPackageGrant = {
  grantId: string;
  expiresAtMs: number;
};
type PickerResult =
  | { kind: "selected"; grantId: string }
  | { kind: "cancelled" }
  | { kind: "unavailable" };

const btnClass =
  "rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2 text-sm font-medium text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]";

async function openModPicker(): Promise<PickerResult> {
  if (typeof window === "undefined" || !isTauriRuntime) {
    return { kind: "unavailable" };
  }

  const grant = await invoke<ModPackageGrant | null>("choose_mod_package_path");
  return grant ? { kind: "selected", grantId: grant.grantId } : { kind: "cancelled" };
}

function ModGlyph({ modId }: { modId: string }) {
  const accentPath =
    modId === "secure-automator"
      ? "M7 13.5 10.2 17 17 8"
      : modId === "document-auditor"
        ? "M8 8h8M8 12h8M8 16h5"
        : modId === "language-adapter"
          ? "M7 8h10M9 8c.8 4 2.2 6.5 6 8M16 8c-.7 4-2.3 6.5-6 8"
          : modId === "strategic-advisor"
            ? "M7 16l3.5-4 3 2L17 8"
            : "M8 9l-2.5 3L8 15M16 9l2.5 3L16 15M13 7l-2 10";

  return (
    <svg
      aria-hidden="true"
      className="h-5 w-5"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.7"
      viewBox="0 0 24 24"
    >
      <path d="M12 3 4.5 7v10L12 21l7.5-4V7L12 3Z" />
      <path d={accentPath} />
    </svg>
  );
}

function XIcon() {
  return (
    <svg
      aria-hidden="true"
      className="h-4 w-4"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeWidth="2"
      viewBox="0 0 24 24"
    >
      <path d="M6 6l12 12M18 6 6 18" />
    </svg>
  );
}

function ToggleSwitch({
  checked,
  label,
  onToggle,
}: {
  checked: boolean;
  label: string;
  onToggle: () => void;
}) {
  return (
    <button
      aria-checked={checked}
      aria-label={label}
      className={`relative h-5 w-9 shrink-0 rounded-full border transition-colors ${
        checked
          ? "border-[var(--accent)] bg-[var(--accent)]"
          : "border-[var(--border-strong)] bg-[var(--background)]"
      }`}
      onClick={onToggle}
      role="switch"
      type="button"
    >
      <span
        aria-hidden="true"
        className={`absolute left-0.5 top-0.5 h-3.5 w-3.5 rounded-full bg-current shadow-sm transition-transform ${
          checked
            ? "translate-x-4 text-[var(--inverse-foreground)]"
            : "translate-x-0 text-[var(--foreground-muted)]"
        }`}
      />
    </button>
  );
}

function ModCard({
  mod,
  onConfigure,
  onToggle,
}: {
  mod: InstalledMod;
  onConfigure: () => void;
  onToggle: () => void;
}) {
  const { t } = useI18n();
  const trust = modTrustPresentation(mod);

  return (
    <article className="flex min-h-[17rem] min-w-0 flex-col rounded-[var(--radius-base)] border border-[var(--border-soft)] bg-[var(--background)] p-4 shadow-[var(--shadow-card)] transition-colors hover:border-[var(--border-strong)] hover:bg-[var(--accent-background)]">
      <div className="flex items-start gap-3">
        <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] text-[var(--foreground)]">
          <ModGlyph modId={mod.id} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <h3 className="truncate text-sm font-semibold text-[var(--foreground)]">
              {mod.name}
            </h3>
            <ModTrustBadge presentation={trust} />
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-[var(--foreground-subtle)]">
            <span>v{mod.version}</span>
            <span aria-hidden="true">/</span>
            <span>{mod.category}</span>
          </div>
        </div>
      </div>

      <p className="mt-4 line-clamp-3 min-h-[3.75rem] text-sm leading-5 text-[var(--foreground-muted)]">
        {mod.description}
      </p>

      <div className="mt-4 flex flex-wrap gap-2">
        <span className="rounded-full bg-[var(--accent-background)] px-2.5 py-1 text-[11px] font-medium text-[var(--foreground-muted)]">
          {mod.author}
        </span>
      </div>

      <div className="mt-auto flex items-center justify-between gap-3 border-t border-[var(--border-soft)] pt-4">
        <div className="flex items-center gap-2">
          <ToggleSwitch
            checked={mod.isActive}
            label={`${mod.isActive ? t("mods.deactivate") : t("mods.activate")} ${mod.name}`}
            onToggle={onToggle}
          />
          <span className="text-xs font-semibold text-[var(--foreground-muted)]">
            {mod.isActive ? t("common.active") : t("common.inactive")}
          </span>
        </div>
        <button
          aria-label={`${t("mods.configure")} ${mod.name}`}
          className="rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-1.5 text-xs font-semibold text-[var(--foreground)] transition-colors hover:bg-[var(--fill-hover)]"
          onClick={onConfigure}
          type="button"
        >
          {t("mods.configure")}
        </button>
      </div>
    </article>
  );
}

function errorMessage(error: unknown, fallback: string) {
  if (error instanceof Error && error.message.trim()) {
    return error.message;
  }
  if (typeof error === "string" && error.trim()) {
    return error;
  }
  return fallback;
}

const CODE_LIKE_PERMISSION_TEXT =
  /(?:`|~~~|<\s*(?:script|style|iframe)\b|javascript:|(?:^|\s)(?:const|let|var|function|class|import|export|return|eval|sudo|curl|wget)\s+|=>|\$\(|\{\s*["']?[\w-]+["']?\s*:)/im;
const SENSITIVE_PERMISSION_TEXT =
  /(?:\b(?:api|access|refresh|session|auth|client)[-_ ]?(?:key|token|secret)\b|\b(?:password|passwd|authorization|bearer|private[-_ ]?key|session[-_ ]?cookie)\b)/i;
const OPAQUE_PERMISSION_TEXT =
  /(?:^|[\s:=])(?:[a-f0-9]{24,}|[a-z0-9+/_-]{36,}={0,2})(?:$|[\s])/i;
const UUID_PERMISSION_TEXT =
  /(?:^|\s)[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}(?:$|\s)/i;

function safePermissionText(value: unknown, maxLength: number) {
  if (typeof value !== "string") return null;
  const raw = value.normalize("NFKC");
  if (raw.length > 4096 || CODE_LIKE_PERMISSION_TEXT.test(raw)) {
    return null;
  }

  const plainText = raw
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/(?:^|\n)\s{0,3}(?:#{1,6}|>|[-+*]|\d+[.)])\s+/g, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/[\u0000-\u001f\u007f]/g, " ")
    .replace(/[\[\]{}|]/g, " ")
    .replace(/[*~]/g, "")
    .replace(/_/g, " ")
    .replace(/\\([\\*_{}\[\]()#+.!~-])/g, "$1")
    .replace(/\s+/g, " ")
    .trim();

  if (
    !plainText ||
    plainText.length > maxLength ||
    !/[\p{L}\p{N}]/u.test(plainText) ||
    SENSITIVE_PERMISSION_TEXT.test(plainText) ||
    OPAQUE_PERMISSION_TEXT.test(plainText) ||
    UUID_PERMISSION_TEXT.test(plainText)
  ) {
    return null;
  }
  return plainText;
}

function safeBundleIdentity(value: unknown) {
  const identity = safePermissionText(value, 96);
  if (
    !identity ||
    /^(?:https?|file):/i.test(identity) ||
    /^(?:[~/\\]|[a-z]:[\\/])/i.test(identity)
  ) {
    return null;
  }
  return identity;
}

function BundlePermissionReview({
  acknowledged,
  activationIntent,
  activating,
  bundle,
  onAcknowledgedChange,
  onActivate,
  onCancel,
}: {
  acknowledged: boolean;
  activationIntent: BundleActivationIntent;
  activating: boolean;
  bundle: CapabilityBundle;
  onAcknowledgedChange: (acknowledged: boolean) => void;
  onActivate: () => void;
  onCancel: () => void;
}) {
  const { t } = useI18n();
  const dialogId = useId();
  const hasDialogTurn = useApprovalDialogTurn(
    true,
    `mod-bundle-review-${dialogId}`,
  );
  if (!hasDialogTurn) return null;

  const safeBundleName = safeBundleIdentity(bundle.name);
  const safePublisherName = safeBundleIdentity(bundle.publisherName);
  const displayBundleName = safeBundleName ?? t("common.unknown");
  const displayPublisherName = safePublisherName ?? t("common.unknown");
  const cannotInstall =
    activating ||
    bundle.reviewState === "revoked" ||
    bundle.integrityState === "modified" ||
    (bundle.reviewState === "unreviewed" && !acknowledged);
  const publisherSummary = bundle.publisherIdentityVerified && safePublisherName
    ? t("mods.made_by_verified", { publisher: displayPublisherName })
    : t("mods.publisher_not_confirmed", { publisher: displayPublisherName });

  return (
    <ApprovalDialogFrame
      description={publisherSummary}
      eyebrow={t("mods.install_mod")}
      footer={<>
        <button
          className={btnClass}
          data-approval-initial-focus
          disabled={activating}
          onClick={onCancel}
          type="button"
        >
          {t("common.cancel")}
        </button>
        <button
          aria-busy={activating}
          className="rounded-[var(--radius-sm)] bg-[var(--accent)] px-4 py-2 text-sm font-semibold text-[var(--inverse-foreground)] disabled:cursor-wait disabled:opacity-40"
          disabled={cannotInstall}
          onClick={onActivate}
          type="button"
        >
          {activating
            ? activationIntent === "install"
              ? t("mods.activating")
              : t("mods.turning_on")
            : activationIntent === "install"
              ? t("mods.install_mod")
              : t("mods.turn_on")}
        </button>
      </>}
      maxWidthClassName="max-w-xl"
      onDismiss={() => { if (!activating) onCancel(); }}
      title={t("mods.review_title", { name: displayBundleName })}
    >
      <div className="mt-5">
        <ModTrustSummary presentation={modTrustPresentation(bundle)} />
      </div>
      <section className="mt-5">
        <h3 className="text-sm font-semibold">{t("mods.what_it_can_do")}</h3>
        <ul className="mt-2 grid gap-2">
          {bundle.capabilities.length ? bundle.capabilities.map((grant, index) => (
            <li
              className="rounded-[var(--radius-sm)] bg-[var(--accent-background)] px-3 py-2 text-sm"
              key={`${grant.capability}-${index}`}
            >
              {t(bundleCapabilitySentenceKey(grant.capability), {
                place: friendlyCapabilityScope(grant.boundedScope, t),
              })}
            </li>
          )) : (
            <li className="text-sm text-[var(--foreground-muted)]">
              {t("mods.no_extra_abilities")}
            </li>
          )}
        </ul>
      </section>
      {bundle.reviewState === "unreviewed" &&
      bundle.integrityState !== "modified" ? (
        <label className="mt-5 flex items-start gap-3 rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-3 text-sm">
          <input
            checked={acknowledged}
            className="mt-0.5"
            disabled={activating}
            onChange={(event) => onAcknowledgedChange(event.target.checked)}
            type="checkbox"
          />
          <span>
            {safePublisherName
              ? t("mods.trust_acknowledgement", { publisher: displayPublisherName })
              : t("mods.trust_unknown_acknowledgement")}
          </span>
        </label>
      ) : null}
    </ApprovalDialogFrame>
  );
}

function ModRemovalConfirmation({
  mod,
  removing,
  onCancel,
  onConfirm,
}: {
  mod: InstalledMod;
  removing: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useI18n();
  const dialogId = useId();
  const hasDialogTurn = useApprovalDialogTurn(
    true,
    `mod-removal-${dialogId}`,
  );
  if (!hasDialogTurn) return null;

  return (
    <ApprovalDialogFrame
      description={t("mods.remove_warning")}
      eyebrow={t("mods.remove_mod")}
      footer={
        <>
          <button
            className={btnClass}
            data-approval-initial-focus
            disabled={removing}
            onClick={onCancel}
            type="button"
          >
            {t("common.cancel")}
          </button>
          <button
            aria-busy={removing}
            className="rounded-[var(--radius-sm)] border border-[var(--destructive)] bg-[var(--background)] px-4 py-2 text-sm font-semibold text-[var(--destructive)] transition-colors hover:bg-[var(--destructive-background)] disabled:cursor-wait disabled:opacity-50"
            disabled={removing}
            onClick={onConfirm}
            type="button"
          >
            {removing ? t("mods.removing") : t("mods.remove_mod")}
          </button>
        </>
      }
      onDismiss={() => {
        if (!removing) onCancel();
      }}
      title={t("mods.remove_title", { name: mod.name })}
    >
      <></>
    </ApprovalDialogFrame>
  );
}

function friendlyCapabilityScope(
  value: unknown,
  t: (key: string) => string,
) {
  if (typeof value !== "string") return t("common.unknown");
  const normalized = value.normalize("NFKC")
    .replace(/[\u0000-\u001f\u007f]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!normalized || normalized.length > 4096) {
    return t("common.unknown");
  }
  try {
    const url = new URL(normalized);
    if (url.protocol === "https:" || url.protocol === "http:") {
      const hostname = safePermissionText(
        url.hostname.replace(/^www\./i, ""),
        160,
      );
      return hostname ?? t("common.unknown");
    }
  } catch {
    // File scopes and named resources are intentionally handled below.
  }
  let candidate = normalized;
  if (/[\\/]/.test(normalized)) {
    candidate = normalized.replace(/[\\/]+$/, "").split(/[\\/]/).at(-1) ?? "";
  }
  const safeCandidate = safePermissionText(candidate, 160);
  if (!safeCandidate) return t("common.unknown");
  return safeCandidate.replace(/[_:.-]+/g, " ").replace(/\s+/g, " ").trim();
}

function bundleCapabilitySentenceKey(value: string) {
  return BUNDLE_CAPABILITY_SENTENCE_KEYS[
    value as CapabilityGrant["capability"]
  ] ?? "mods.capability_sentences.other";
}

export function ModsScreen() {
  const { t } = useI18n();
  const [mods, setMods] = useState<InstalledMod[]>([]);
  const [loading, setLoading] = useState(true);
  const [selectedModId, setSelectedModId] = useState<string | null>(null);
  const [notice, setNotice] = useState<{
    tone: "success" | "info" | "error";
    message: string;
  } | null>(null);
  const [removingModId, setRemovingModId] = useState<string | null>(null);
  const [pendingRemovalMod, setPendingRemovalMod] = useState<InstalledMod | null>(null);
  const [pendingBundle, setPendingBundle] = useState<CapabilityBundle | null>(null);
  const [pendingBundleIntent, setPendingBundleIntent] =
    useState<BundleActivationIntent>("install");
  const [acknowledged, setAcknowledged] = useState(false);
  const [activatingBundle, setActivatingBundle] = useState(false);
  const activatingBundleRef = useRef(false);
  const drawerRef = useRef<HTMLElement | null>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const activeCount = useMemo(
    () => mods.filter((mod) => mod.isActive).length,
    [mods],
  );
  const selectedMod = selectedModId
    ? mods.find((mod) => mod.id === selectedModId) ?? null
    : null;

  useEffect(() => {
    if (!notice || notice.tone !== "success") {
      return;
    }

    const timeoutId = window.setTimeout(() => {
      setNotice((current) => (current === notice ? null : current));
    }, 4_000);

    return () => window.clearTimeout(timeoutId);
  }, [notice]);

  useEffect(() => {
    let active = true;
    invoke<InstalledMod[]>("list_installed_mods")
      .then((data) => {
        if (active) {
          setMods(data);
          setLoading(false);
        }
      })
      .catch((error: unknown) => {
        console.error("Failed to load OOMU mods", error);
        if (active) {
          setNotice({
            tone: "error",
            message: errorMessage(error, t("mods.errors.load")),
          });
          setLoading(false);
        }
      });
    return () => {
      active = false;
    };
  }, [t]);

  // Treat the detail drawer as a real dialog: focus in on open, trap Tab, Escape to close, restore focus on close.
  useEffect(() => {
    if (!selectedModId) {
      restoreFocusRef.current?.focus?.();
      restoreFocusRef.current = null;
      return;
    }

    const drawer = drawerRef.current;
    if (!drawer) {
      return;
    }

    const getFocusable = () =>
      Array.from(
        drawer.querySelectorAll<HTMLElement>(
          'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((element) => !element.hasAttribute("disabled"));

    getFocusable()[0]?.focus();

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        setSelectedModId(null);
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const focusable = getFocusable();
      if (focusable.length === 0) {
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [selectedModId]);

  async function handleToggleMod(modId: string, currentActiveState: boolean) {
    setNotice(null);
    try {
      const nextState = !currentActiveState;
      const mod = mods.find((candidate) => candidate.id === modId);
      if (nextState && mod && !mod.isBuiltIn) {
        const inspected = await invoke<CapabilityBundle>(
          "inspect_capability_bundle",
          { request: { modId, projectIds: [] } },
        );
        if (
          inspected.reviewState === "reviewed" &&
          inspected.integrityState === "verified"
        ) {
          await activateBundle(inspected, false, "enable");
          return;
        }
        setPendingBundle(inspected);
        setPendingBundleIntent("enable");
        setAcknowledged(false);
        return;
      }
      await invoke("set_mod_active_state", { modId, active: nextState });
      setMods((currentMods) =>
        currentMods.map((mod) =>
          mod.id === modId ? { ...mod, isActive: nextState } : mod,
        ),
      );
    } catch (error: unknown) {
      setNotice({
        tone: "error",
        message: errorMessage(error, t("mods.errors.state")),
      });
    }
  }

  async function handleInstallMod() {
    let result: PickerResult;
    try {
      result = await openModPicker();
    } catch (error: unknown) {
      setNotice({
        tone: "error",
        message: errorMessage(error, t("mods.errors.picker")),
      });
      return;
    }
    if (result.kind === "cancelled") {
      return;
    }
    if (result.kind === "unavailable") {
      setNotice({
        tone: "error",
        message: t("mods.errors.native_only"),
      });
      return;
    }

    try {
      const newlyInstalled = await invoke<InstalledMod>("install_mod_from_path", {
        grantId: result.grantId,
      });
      setMods((currentMods) => [
        ...currentMods.filter((mod) => mod.id !== newlyInstalled.id),
        newlyInstalled,
      ]);
      const inspected=await invoke<CapabilityBundle>("inspect_capability_bundle",{request:{modId:newlyInstalled.id,projectIds:[]}});
      if (
        inspected.reviewState === "reviewed" &&
        inspected.integrityState === "verified"
      ) {
        await activateBundle(inspected, false, "install");
      } else {
        setPendingBundle(inspected);
        setPendingBundleIntent("install");
        setAcknowledged(false);
      }
    } catch (error: unknown) {
      setNotice({
        tone: "error",
        message: errorMessage(error, t("mods.errors.install")),
      });
    }
  }

  async function activateBundle(
    bundle: CapabilityBundle,
    acknowledgeUnreviewed: boolean,
    intent: BundleActivationIntent,
  ) {
    const safeName = safeBundleIdentity(bundle.name) ?? t("common.unknown");
    await invoke("activate_capability_bundle", {
      request: {
        bundleId: bundle.bundleId,
        packageVersion: bundle.packageVersion,
        acknowledgeUnreviewed,
      },
    });
    setMods((current) =>
      current.map((mod) =>
        mod.id === bundle.modId ? { ...mod, isActive: true } : mod,
      ),
    );
    setPendingBundle(null);
    setNotice({
      tone: "success",
      message:
        intent === "install"
          ? t("mods.installed_success", { name: safeName })
          : t("mods.enabled_success", { name: safeName }),
    });
  }

  async function activatePendingBundle() {
    if(activatingBundleRef.current||!pendingBundle)return;
    activatingBundleRef.current=true;
    setActivatingBundle(true);setNotice(null);
    try {
      const reviewed=await invoke<CapabilityBundle>("inspect_capability_bundle",{request:{modId:pendingBundle.modId,projectIds:[]}});
      if (reviewed.reviewState==="revoked"||reviewed.integrityState==="modified") {
        setPendingBundle(reviewed);
        setAcknowledged(false);
        return;
      }
      await activateBundle(
        reviewed,
        reviewed.reviewState !== "reviewed" && acknowledged,
        pendingBundleIntent,
      );
    } catch(error:unknown) { setNotice({tone:"error",message:errorMessage(error,t("mods.errors.activate_bundle"))}); }
    finally { activatingBundleRef.current=false; setActivatingBundle(false); }
  }

  async function handleRemoveMod(mod: InstalledMod) {
    setRemovingModId(mod.id);
    setNotice(null);
    try {
      await invoke("uninstall_mod", { modId: mod.id });
      const installedMods = await invoke<InstalledMod[]>("list_installed_mods");
      if (installedMods.some((installed) => installed.id === mod.id)) {
        throw new Error(t("mods.errors.remove"));
      }
      setMods(installedMods);
      setSelectedModId(null);
      setPendingRemovalMod(null);
      setNotice({
        tone: "success",
        message: t("mods.removed", { name: mod.name }),
      });
    } catch (error: unknown) {
      setNotice({
        tone: "error",
        message: errorMessage(error, t("mods.errors.remove")),
      });
    } finally {
      setRemovingModId(null);
    }
  }

  return (
    <section className="relative flex h-full min-h-0 w-full overflow-hidden bg-[var(--background)] text-[var(--foreground)]">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <header className="flex h-12 shrink-0 items-center justify-between border-b border-[var(--border-soft)] px-5">
          <span className="rounded-full bg-[var(--accent-background)] px-2.5 py-1 text-xs font-semibold text-[var(--foreground-muted)]">
            {t("mods.active_count", { count: activeCount })}
          </span>
          <div className="flex items-center gap-2">
            <button className={btnClass} onClick={handleInstallMod} type="button">
              {t("mods.install_mod")}
            </button>
            <button
              className={btnClass}
              onClick={() => void invoke("open_oomu_marketplace")}
              type="button"
            >
              {t("mods.browse_marketplace")}
            </button>
          </div>
        </header>

        <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-5 py-5">
          {notice ? (
            <div
              className={`mb-4 flex w-full items-center gap-3 rounded-[var(--radius-md)] border bg-[var(--background)] px-4 py-3 text-sm font-medium shadow-[var(--shadow-card)] ${
                notice.tone === "success"
                  ? "border-[var(--success)] text-[var(--foreground)]"
                  : notice.tone === "error"
                    ? "border-[var(--destructive)] text-[var(--destructive)]"
                    : "border-[var(--border-strong)] text-[var(--foreground)]"
              }`}
              data-oomu-mod-notice={notice.tone}
              role={notice.tone === "error" ? "alert" : "status"}
            >
              <span>{notice.message}</span>
            </div>
          ) : null}
          {loading ? (
            <p className="py-16 text-center text-sm text-[var(--foreground-muted)]">
              {t("mods.loading")}
            </p>
          ) : mods.length > 0 ? (
            <div className="grid grid-cols-2 gap-4 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
              {mods.map((mod) => (
                <ModCard
                  key={mod.id}
                  mod={mod}
                  onConfigure={() => {
                    restoreFocusRef.current = (document.activeElement as HTMLElement) ?? null;
                    setSelectedModId(mod.id);
                  }}
                  onToggle={() => handleToggleMod(mod.id, mod.isActive)}
                />
              ))}
            </div>
          ) : (
            <div className="flex flex-col items-center gap-4 rounded-[var(--radius-base)] border border-[var(--border-soft)] bg-[var(--background)] px-6 py-16 text-center">
              <span className="flex h-12 w-12 items-center justify-center rounded-[var(--radius-base)] border border-[var(--border-soft)] bg-[var(--accent-background)] text-[var(--foreground-muted)]">
                <ModGlyph modId="" />
              </span>
              <div>
                <p className="text-sm font-semibold text-[var(--foreground)]">
                  {t("mods.empty_title")}
                </p>
                <p className="mt-1 text-sm text-[var(--foreground-muted)]">
                  {t("mods.empty_description")}
                </p>
              </div>
              <button className={btnClass} onClick={handleInstallMod} type="button">
                {t("mods.install_mod")}
              </button>
            </div>
          )}
        </div>
      </div>

      {selectedMod ? (
        <button
          aria-label={t("mods.close_configuration")}
          className="fixed inset-x-0 bottom-0 top-12 z-20 bg-black/10 lg:hidden"
          onClick={() => setSelectedModId(null)}
          type="button"
        />
      ) : null}

      <aside
        aria-hidden={!selectedMod}
        aria-labelledby="mod-drawer-title"
        aria-modal="true"
        className={`fixed bottom-0 right-0 top-12 z-30 flex w-[min(24rem,100vw)] flex-col border-l border-[var(--border-soft)] bg-[var(--background)] shadow-[var(--shadow-raised)] transition-transform duration-200 ${
          selectedMod ? "translate-x-0" : "translate-x-full"
        }`}
        ref={drawerRef}
        role="dialog"
      >
        {selectedMod ? (
          <>
            <div className="flex h-12 shrink-0 items-center justify-between border-b border-[var(--border-soft)] px-5">
              <div className="min-w-0">
                <h2
                  className="truncate text-sm font-semibold text-[var(--foreground)]"
                  id="mod-drawer-title"
                >
                  {selectedMod.name}
                </h2>
                <p className="text-xs text-[var(--foreground-muted)]">
                  v{selectedMod.version} {t("mods.by_author", { author: selectedMod.author })}
                </p>
              </div>
              <button
                aria-label={t("common.close")}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[var(--radius-sm)] text-[var(--foreground-muted)] transition-colors hover:bg-[var(--fill-hover)] hover:text-[var(--foreground)]"
                onClick={() => {
                  setSelectedModId(null);
                }}
                type="button"
              >
                <XIcon />
              </button>
            </div>

            <div className="custom-scrollbar flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto px-5 py-5">
              <section className="rounded-[var(--radius-base)] border border-[var(--border-soft)] bg-[var(--accent-background)] p-4">
                <div className="flex items-center gap-3">
                  <span className="flex h-10 w-10 items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)]">
                    <ModGlyph modId={selectedMod.id} />
                  </span>
                  <div>
                    <p className="text-sm font-semibold text-[var(--foreground)]">
                      {selectedMod.category}
                    </p>
                    <p className="text-xs text-[var(--foreground-muted)]">
                      {selectedMod.packageSize} / {selectedMod.lastUpdated}
                    </p>
                  </div>
                </div>
                <p className="mt-4 text-sm leading-6 text-[var(--foreground-muted)]">
                  {selectedMod.description}
                </p>
              </section>

              <ModTrustSummary presentation={modTrustPresentation(selectedMod)} />

              <section>
                <div className="flex items-center justify-between gap-3">
                  <h3 className="text-xs font-semibold text-[var(--foreground-muted)]">
                    {t("mods.global_activation")}
                  </h3>
                  <ToggleSwitch
                    checked={selectedMod.isActive}
                    label={`${selectedMod.isActive ? t("mods.deactivate") : t("mods.activate")} ${selectedMod.name}`}
                    onToggle={() => handleToggleMod(selectedMod.id, selectedMod.isActive)}
                  />
                </div>
                <p className="mt-2 text-sm leading-6 text-[var(--foreground-muted)]">
                  {selectedMod.isActive
                    ? t("mods.active_description")
                    : t("mods.inactive_description")}
                </p>
              </section>

              <section>
                <h3 className="text-xs font-semibold text-[var(--foreground-muted)]">
                  {t("mods.permissions")}
                </h3>
                <div className="mt-3 flex flex-col gap-2">
                  {selectedMod.permissions.map((permission) => (
                    <div
                      className="rounded-[var(--radius-sm)] border border-[var(--border-soft)] bg-[var(--background)] p-3"
                      key={permission.label}
                    >
                      <p className="text-sm font-semibold text-[var(--foreground)]">
                        {permission.label}
                      </p>
                      <p className="mt-1 text-xs leading-5 text-[var(--foreground-muted)]">
                        {permission.detail}
                      </p>
                    </div>
                  ))}
                </div>
              </section>

              <section>
                <h3 className="text-xs font-semibold text-[var(--foreground-muted)]">
                  {t("mods.network_endpoints")}
                </h3>
                <div className="mt-3 flex flex-wrap gap-2">
                  {selectedMod.endpoints.map((endpoint) => (
                    <span
                      className="rounded-full border border-[var(--border-soft)] bg-[var(--accent-background)] px-2.5 py-1 text-xs font-medium text-[var(--foreground-muted)]"
                      key={endpoint}
                    >
                      {endpoint}
                    </span>
                  ))}
                </div>
              </section>

              <div className="mt-auto rounded-[var(--radius-base)] border border-[var(--border-soft)] bg-[var(--background)] p-4">
                <h3 className="text-xs font-semibold text-[var(--foreground-muted)]">
                  {t("mods.remove_mod")}
                </h3>
                <p className="mt-2 text-xs leading-5 text-[var(--foreground-muted)]">
                  {t("mods.remove_description")}
                </p>
                <button
                  className="mt-4 w-full rounded-[var(--radius-sm)] border border-[var(--destructive)] bg-[var(--background)] px-3 py-2 text-sm font-semibold text-[var(--destructive)] transition-colors hover:bg-[var(--destructive-background)] disabled:cursor-wait disabled:opacity-60"
                  disabled={removingModId === selectedMod.id}
                  onClick={() => setPendingRemovalMod(selectedMod)}
                  type="button"
                >
                  {t("mods.remove_mod")}
                </button>
              </div>
            </div>
          </>
        ) : null}
      </aside>

      {pendingBundle ? (
        <BundlePermissionReview
          acknowledged={acknowledged}
          activationIntent={pendingBundleIntent}
          activating={activatingBundle}
          bundle={pendingBundle}
          onAcknowledgedChange={setAcknowledged}
          onActivate={() => void activatePendingBundle()}
          onCancel={() => {
            if (!activatingBundleRef.current) setPendingBundle(null);
          }}
        />
      ) : null}

      {pendingRemovalMod ? (
        <ModRemovalConfirmation
          mod={pendingRemovalMod}
          onCancel={() => setPendingRemovalMod(null)}
          onConfirm={() => void handleRemoveMod(pendingRemovalMod)}
          removing={removingModId === pendingRemovalMod.id}
        />
      ) : null}

    </section>
  );
}
