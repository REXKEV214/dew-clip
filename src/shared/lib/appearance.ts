export const APPEARANCE_SETTING_KEY = "app.appearance";
export const APPEARANCE_CHANGED_EVENT = "appearance-changed";
const APPEARANCE_MIRROR_KEY = "dewclip_appearance";

export type EffectiveTheme = "light" | "dark";
export type AppearancePreference = "system" | EffectiveTheme;

export interface AppearanceState {
  preference: AppearancePreference;
  effective: EffectiveTheme;
}

export const normalizeAppearance = (value: string | null | undefined): AppearancePreference =>
  value === "light" || value === "dark" ? value : "system";

export const resolveInitialTheme = (preference: AppearancePreference): EffectiveTheme => {
  if (preference !== "system") return preference;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
};

export const applyEffectiveTheme = (theme: EffectiveTheme, root = document.documentElement) => {
  root.dataset.theme = theme;
  root.style.colorScheme = theme;
};

export const mirrorAppearance = (preference: AppearancePreference) => {
  try {
    window.localStorage.setItem(APPEARANCE_MIRROR_KEY, preference);
  } catch {
    // The WebView uses its native appearance before the backend state arrives.
  }
};

export const mirroredAppearance = (): AppearancePreference => {
  try {
    return normalizeAppearance(window.localStorage.getItem(APPEARANCE_MIRROR_KEY));
  } catch {
    return "system";
  }
};
