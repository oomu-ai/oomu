"use client";

import { useEffect, useState } from "react";
import { invoke } from "@/lib/invoke";
import { useI18n } from "@/context/I18nContext";
import { SettingsHeader } from "./SettingsHeader";

type UserPersonalityProfile = {
  display_name: string;
  pronouns: string;
  role_or_work: string;
  location_timezone: string;
  bio_context: string;
  should_know: string;
  should_respond: string;
  areas_of_expertise: string;
  current_priorities: string;
  languages: string;
  interests_preferences: string;
  boundaries: string;
  default_tone: string;
  response_length: string;
  formatting_style: string;
  conversation_defaults: string[];
  signature?: unknown;
  updated_at_ms: number;
};

type TabId = "identity" | "instructions" | "context" | "style";

const TABS: { id: TabId; labelKey: string }[] = [
  { id: "identity", labelKey: "user_config.tabs.identity" },
  { id: "instructions", labelKey: "user_config.tabs.instructions" },
  { id: "context", labelKey: "user_config.tabs.context" },
  { id: "style", labelKey: "user_config.tabs.style" },
];

// The persisted value stays the canonical English string so existing saved
// profiles keep working; only the visible label follows the active locale.
const CONVERSATION_DEFAULTS: { value: string; labelKey: string }[] = [
  { value: "Ask before making big assumptions", labelKey: "user_config.defaults.ask_first" },
  { value: "Show uncertainty when facts are unclear", labelKey: "user_config.defaults.show_uncertainty" },
  { value: "Prefer actionable next steps", labelKey: "user_config.defaults.actionable" },
  { value: "Use examples from my work context", labelKey: "user_config.defaults.work_examples" },
  { value: "Remember accessibility preferences", labelKey: "user_config.defaults.accessibility" },
];

const TONE_OPTIONS = ["Direct", "Warm", "Analytical", "Creative", "Coaching"];
const LENGTH_OPTIONS = ["Concise", "Balanced", "Detailed"];
const STYLE_OPTIONS = [
  "Plain prose",
  "Bulleted summaries",
  "Step-by-step",
  "Technical notes",
];

export function UserConfigPanel() {
  const { t } = useI18n();
  const [activeTab, setActiveTab] = useState<TabId>("identity");
  const [profile, setProfile] = useState<UserPersonalityProfile>({
    display_name: "",
    pronouns: "",
    role_or_work: "",
    location_timezone: "",
    bio_context: "",
    should_know: "",
    should_respond: "",
    areas_of_expertise: "",
    current_priorities: "",
    languages: "",
    interests_preferences: "",
    boundaries: "",
    default_tone: "",
    response_length: "",
    formatting_style: "",
    conversation_defaults: [],
    signature: undefined,
    updated_at_ms: 0,
  });
  const [saveState, setSaveState] = useState(() => t("user_config.state.ready"));

  const inputClass =
    "mt-2 w-full rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2.5 text-sm text-[var(--foreground)] outline-none transition-colors placeholder:text-[var(--foreground-subtle)] focus:bg-[var(--accent-background)]";
  const textareaClass =
    "mt-2 flex-1 w-full resize-none rounded-[var(--radius-sm)] border border-[var(--border-strong)] bg-[var(--background)] px-3 py-2.5 text-sm leading-6 text-[var(--foreground)] outline-none transition-colors placeholder:text-[var(--foreground-subtle)] focus:bg-[var(--accent-background)]";
  const labelClass = "text-xs font-medium text-[var(--foreground-muted)]";
  const tabButtonClass = (isActive: boolean) =>
    `w-full text-left px-5 py-3 text-sm font-medium border-b border-[var(--border-strong)] transition-colors ${
      isActive
        ? "bg-[var(--fill-selected)] text-[var(--foreground)]"
        : "text-[var(--foreground)] hover:bg-[var(--fill-hover)]"
    }`;

  useEffect(() => {
    let cancelled = false;
    void invoke<UserPersonalityProfile | null>("get_user_personality_profile")
      .then((savedProfile) => {
        if (!cancelled && savedProfile) {
          setProfile((current) => ({ ...current, ...savedProfile, signature: savedProfile.signature }));
          setSaveState(t("user_config.state.loaded"));
        }
      })
      .catch(() => setSaveState(t("user_config.state.load_error")));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const updateProfileField = (field: keyof typeof profile, value: string) => {
    setProfile((current) => ({ ...current, [field]: value }));
    setSaveState(t("user_config.state.unsaved"));
  };

  const toggleConversationDefault = (item: string) => {
    setProfile((current) => {
      const selected = current.conversation_defaults.includes(item)
        ? current.conversation_defaults.filter((entry) => entry !== item)
        : [...current.conversation_defaults, item];
      return { ...current, conversation_defaults: selected };
    });
    setSaveState(t("user_config.state.unsaved"));
  };

  const saveUserProfile = async () => {
    setSaveState(t("user_config.state.saving"));
    try {
      const savedProfile = await invoke<UserPersonalityProfile>("save_user_personality_profile", { profile });
      setProfile((current) => ({ ...current, ...savedProfile, signature: savedProfile.signature }));
      setSaveState(t("user_config.state.saved"));
    } catch {
      setSaveState(t("user_config.state.save_error"));
    }
  };

  return (
    <section className="flex h-[calc(100vh-7rem)] flex-col">
      <div className="shrink-0">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <SettingsHeader title={t("user_config.title")} />
          <button
            className="rounded-[var(--radius-sm)] bg-[var(--inverse-background)] px-3 py-2 text-sm font-semibold text-[var(--inverse-foreground)] transition-colors hover:bg-[var(--accent-hover)]"
            onClick={saveUserProfile}
            type="button"
          >
            {t("user_config.save")}
          </button>
        </div>
        <p className="mb-5 mt-2 text-sm leading-6 text-[var(--foreground-muted)]">
          {t("user_config.description")}
        </p>
        <p className="mb-5 text-xs text-[var(--foreground-muted)]">
          {saveState}
        </p>
      </div>

      <div className="flex min-h-0 flex-1 overflow-hidden rounded-[var(--radius-md)] border border-[var(--border-strong)] bg-[var(--background)]">
        <aside className="flex w-60 shrink-0 flex-col border-r border-[var(--border-strong)]">
          {TABS.map((tab) => (
            <button
              className={tabButtonClass(activeTab === tab.id)}
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              type="button"
            >
              {t(tab.labelKey)}
            </button>
          ))}
        </aside>

        <div className="flex min-h-0 flex-1 flex-col overflow-hidden p-6">
          {activeTab === "identity" && (
            <div className="flex h-full flex-col gap-5">
              <div className="grid shrink-0 gap-4 md:grid-cols-2 lg:grid-cols-4">
                <label>
                  <span className={labelClass}>{t("user_config.fields.display_name")}</span>
                  <input className={inputClass} onChange={(event) => updateProfileField("display_name", event.target.value)} placeholder={t("user_config.placeholders.display_name")} type="text" value={profile.display_name} />
                </label>
                <label>
                  <span className={labelClass}>{t("user_config.fields.pronouns")}</span>
                  <input className={inputClass} onChange={(event) => updateProfileField("pronouns", event.target.value)} placeholder={t("user_config.placeholders.optional")} type="text" value={profile.pronouns} />
                </label>
                <label>
                  <span className={labelClass}>{t("user_config.fields.role_or_work")}</span>
                  <input className={inputClass} onChange={(event) => updateProfileField("role_or_work", event.target.value)} placeholder={t("user_config.placeholders.role_or_work")} type="text" value={profile.role_or_work} />
                </label>
                <label>
                  <span className={labelClass}>{t("user_config.fields.location_timezone")}</span>
                  <input className={inputClass} onChange={(event) => updateProfileField("location_timezone", event.target.value)} placeholder={t("user_config.placeholders.location_timezone")} type="text" value={profile.location_timezone} />
                </label>
              </div>
              <label className="flex min-h-0 flex-1 flex-col">
                <span className={labelClass}>{t("user_config.fields.bio_context")}</span>
                <textarea
                  className={textareaClass}
                  onChange={(event) => updateProfileField("bio_context", event.target.value)}
                  placeholder={t("user_config.placeholders.bio_context")}
                  value={profile.bio_context}
                />
              </label>
            </div>
          )}

          {activeTab === "instructions" && (
            <div className="flex h-full flex-col gap-5">
              <label className="flex min-h-0 flex-1 flex-col">
                <span className={labelClass}>{t("user_config.fields.should_know")}</span>
                <textarea
                  className={textareaClass}
                  onChange={(event) => updateProfileField("should_know", event.target.value)}
                  placeholder={t("user_config.placeholders.should_know")}
                  value={profile.should_know}
                />
              </label>
              <label className="flex min-h-0 flex-1 flex-col">
                <span className={labelClass}>{t("user_config.fields.should_respond")}</span>
                <textarea
                  className={textareaClass}
                  onChange={(event) => updateProfileField("should_respond", event.target.value)}
                  placeholder={t("user_config.placeholders.should_respond")}
                  value={profile.should_respond}
                />
              </label>
            </div>
          )}

          {activeTab === "context" && (
            <div className="flex h-full flex-col gap-5">
              <div className="grid shrink-0 gap-4 md:grid-cols-2 lg:grid-cols-3">
                <label>
                  <span className={labelClass}>{t("user_config.fields.areas_of_expertise")}</span>
                  <input className={inputClass} onChange={(event) => updateProfileField("areas_of_expertise", event.target.value)} placeholder={t("user_config.placeholders.areas_of_expertise")} type="text" value={profile.areas_of_expertise} />
                </label>
                <label>
                  <span className={labelClass}>{t("user_config.fields.current_priorities")}</span>
                  <input className={inputClass} onChange={(event) => updateProfileField("current_priorities", event.target.value)} placeholder={t("user_config.placeholders.current_priorities")} type="text" value={profile.current_priorities} />
                </label>
                <label>
                  <span className={labelClass}>{t("user_config.fields.languages")}</span>
                  <input className={inputClass} onChange={(event) => updateProfileField("languages", event.target.value)} placeholder={t("user_config.placeholders.languages")} type="text" value={profile.languages} />
                </label>
              </div>
              <div className="grid min-h-0 flex-1 gap-4 md:grid-cols-2">
                <label className="flex h-full flex-col">
                  <span className={labelClass}>{t("user_config.fields.interests_preferences")}</span>
                  <textarea className={textareaClass} onChange={(event) => updateProfileField("interests_preferences", event.target.value)} placeholder={t("user_config.placeholders.interests_preferences")} value={profile.interests_preferences} />
                </label>
                <label className="flex h-full flex-col">
                  <span className={labelClass}>{t("user_config.fields.boundaries")}</span>
                  <textarea className={textareaClass} onChange={(event) => updateProfileField("boundaries", event.target.value)} placeholder={t("user_config.placeholders.boundaries")} value={profile.boundaries} />
                </label>
              </div>
            </div>
          )}

          {activeTab === "style" && (
            <div className="flex h-full flex-col gap-5">
              <div className="grid shrink-0 gap-4 md:grid-cols-3">
                <label>
                  <span className={labelClass}>{t("user_config.fields.default_tone")}</span>
                  <select className={inputClass} onChange={(event) => updateProfileField("default_tone", event.target.value)} value={profile.default_tone}>
                    <option value="" disabled>{t("user_config.select_tone")}</option>
                    {TONE_OPTIONS.map((tone) => (
                      <option key={tone} value={tone}>
                        {t(`user_config.tone.${tone.toLowerCase()}`)}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  <span className={labelClass}>{t("user_config.fields.response_length")}</span>
                  <select className={inputClass} onChange={(event) => updateProfileField("response_length", event.target.value)} value={profile.response_length}>
                    <option value="" disabled>{t("user_config.select_length")}</option>
                    {LENGTH_OPTIONS.map((length) => (
                      <option key={length} value={length}>
                        {t(`user_config.length.${length.toLowerCase()}`)}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  <span className={labelClass}>{t("user_config.fields.formatting_style")}</span>
                  <select className={inputClass} onChange={(event) => updateProfileField("formatting_style", event.target.value)} value={profile.formatting_style}>
                    <option value="" disabled>{t("user_config.select_style")}</option>
                    {STYLE_OPTIONS.map((style) => (
                      <option key={style} value={style}>
                        {t(`user_config.style_option.${styleOptionSlug(style)}`)}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
              <div className="mt-2 flex-1 overflow-y-auto">
                <h3 className={`${labelClass} mb-4 block`}>{t("user_config.conversation_defaults")}</h3>
                <div className="flex flex-col gap-3">
                  {CONVERSATION_DEFAULTS.map((item) => (
                    <label className="flex items-start gap-3 text-sm leading-5 text-[var(--foreground-muted)]" key={item.value}>
                      <input
                        checked={profile.conversation_defaults.includes(item.value)}
                        className="mt-1 h-4 w-4 accent-[var(--accent)]"
                        onChange={() => toggleConversationDefault(item.value)}
                        type="checkbox"
                      />
                      <span>{t(item.labelKey)}</span>
                    </label>
                  ))}
                </div>
              </div>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}

function styleOptionSlug(style: string) {
  return style.toLowerCase().replace(/[^a-z]+/g, "_");
}
