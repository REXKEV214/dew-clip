import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SettingsStore } from "../app/useSettings";
import type { AppCleanupPolicy } from "./types";
import { IGNORED_APPS_KEY, addIgnoredApp, canonicalizeAppPaths, listIgnoredApps, parseAppPolicies, removeIgnoredApp } from "./utils/ignoredApps";

export const useIgnoredApps = ({ values, remember, t }: SettingsStore, pushToast: (message: string) => void) => {
  const raw = values[IGNORED_APPS_KEY];
  const latest = useRef(raw);
  const pending = useRef(Promise.resolve());
  useEffect(() => { latest.current = raw; }, [raw]);

  const ignored = useMemo(() => listIgnoredApps(parseAppPolicies(raw)), [raw]);
  const ignoredPaths = useMemo(() => ignored.map((app) => app.appPath ?? "").filter(Boolean), [ignored]);
  const [canonicalIgnored, setCanonicalIgnored] = useState<{ source: string[]; paths: string[] }>();
  useEffect(() => {
    if (!ignoredPaths.length) return;
    let active = true;
    void canonicalizeAppPaths(ignoredPaths).then((paths) => {
      if (active) setCanonicalIgnored({ source: ignoredPaths, paths });
    }).catch(console.error);
    return () => { active = false; };
  }, [ignoredPaths]);

  const update = useCallback((mutate: (policies: AppCleanupPolicy[]) => AppCleanupPolicy[] | Promise<AppCleanupPolicy[]>, toast?: string) => {
    pending.current = pending.current.then(async () => {
      const previous = parseAppPolicies(latest.current);
      const next = await mutate(previous);
      if (next === previous) {
        pushToast(t("ignored_apps_already_added"));
        return;
      }
      const policies = JSON.stringify(next);
      await invoke("set_app_cleanup_policies", { policies });
      latest.current = policies;
      remember(IGNORED_APPS_KEY, policies);
      if (toast) pushToast(toast);
    }).catch((error) => pushToast(`${t("ignored_apps_save_failed")} ${String(error)}`));
  }, [pushToast, remember, t]);

  const add = useCallback((name: string, path: string, toast?: string) => {
    update(async (policies) => {
      const paths = [...new Set([path, ...policies.map((policy) => policy.appPath ?? "").filter(Boolean)])];
      const canonical = await canonicalizeAppPaths(paths);
      return addIgnoredApp(policies, { name, path }, new Map(paths.map((value, index) => [value, canonical[index]])));
    }, toast);
  }, [update]);
  const remove = (policy: AppCleanupPolicy) => update((policies) => removeIgnoredApp(policies, policy));
  const excludePaths = canonicalIgnored?.source === ignoredPaths ? canonicalIgnored.paths : ignoredPaths;
  return { ignored, excludePaths, add, remove };
};
