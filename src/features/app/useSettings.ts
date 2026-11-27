import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { translations, type TranslationKey } from "../../locales";
import type { Locale, Translate } from "../../shared/types";
import { isTauriRuntime } from "../../shared/lib/tauriRuntime";
import { ACCENT_SETTING_KEY, applyAccent, mirrorAccent, normalizeAccent } from "../../shared/lib/accent";
import {
  APPEARANCE_CHANGED_EVENT, APPEARANCE_SETTING_KEY, applyEffectiveTheme, mirrorAppearance,
  type AppearanceState,
} from "../../shared/lib/appearance";

const LANGUAGE_MIRROR_KEY = "dewclip_language";

const systemLanguage = (): Locale => (navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en");

export interface SettingsStore {
  ready: boolean;
  values: Record<string, string>;
  language: Locale;
  t: Translate;
  flag: (key: string, fallback?: boolean) => boolean;
  /** Saves a setting and keeps the window in step with it. */
  save: (key: string, value: string) => void;
  /** For settings saved by a dedicated backend command. */
  remember: (key: string, value: string) => void;
}

export const useSettings = (): SettingsStore => {
  const [values, setValues] = useState<Record<string, string>>({});
  const [ready, setReady] = useState(false);

  useEffect(() => {
    if (!isTauriRuntime()) {
      setReady(true);
      return;
    }
    invoke<Record<string, string>>("get_settings")
      .then((loaded) => {
        // An appearance event may have arrived while the settings were loading.
        setValues((current) => ({ ...loaded, ...current }));
        setReady(true);
      })
      .catch((error) => {
        // The defaults still give a working window; waiting left it blank.
        console.error("Settings failed to load", error);
        setReady(true);
      });
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    let receivedEvent = false;

    const applyState = ({ preference, effective }: AppearanceState) => {
      if (disposed) return;
      applyEffectiveTheme(effective);
      mirrorAppearance(preference);
      setValues((current) => current[APPEARANCE_SETTING_KEY] === preference ? current : {
        ...current,
        [APPEARANCE_SETTING_KEY]: preference,
      });
    };
    const subscription = listen<AppearanceState>(APPEARANCE_CHANGED_EVENT, ({ payload }) => {
      if (disposed) return;
      receivedEvent = true;
      applyState(payload);
    });
    void subscription.then(async () => {
      if (disposed) return;
      const state = await invoke<AppearanceState>("get_appearance_state");
      if (!receivedEvent) applyState(state);
    }).catch((error) => console.error("Appearance failed to load", error));

    return () => {
      disposed = true;
      void subscription.then((unlisten) => unlisten(), () => {});
    };
  }, []);

  const language: Locale = useMemo(() => {
    const saved = values["app.language"];
    return saved === "zh" || saved === "en" ? saved : systemLanguage();
  }, [values]);

  useEffect(() => {
    try {
      localStorage.setItem(LANGUAGE_MIRROR_KEY, language);
    } catch {
      // Only the crash screen reads this.
    }
    document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
  }, [language]);

  // Until settings load, the colour mirrored last time (applied in main.tsx) stays.
  const accent = ready ? normalizeAccent(values[ACCENT_SETTING_KEY]) : null;
  useEffect(() => {
    if (!accent) return;
    applyAccent(accent);
    mirrorAccent(accent);
  }, [accent]);

  const t: Translate = useCallback(
    (key) => translations[language][key as TranslationKey] ?? translations.en[key as TranslationKey] ?? key,
    [language]
  );

  const flag = useCallback(
    (key: string, fallback = false) => {
      const value = values[key];
      return value === undefined ? fallback : value === "true";
    },
    [values]
  );

  const remember = useCallback((key: string, value: string) => {
    setValues((prev) => ({ ...prev, [key]: value }));
  }, []);

  const save = useCallback(
    (key: string, value: string) => {
      if (key !== APPEARANCE_SETTING_KEY) remember(key, value);
      invoke("save_setting", { key, value }).catch((error) => console.error(`Saving ${key} failed`, error));
    },
    [remember]
  );

  return { ready, values, language, t, flag, save, remember };
};
