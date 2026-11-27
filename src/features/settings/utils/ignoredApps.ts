import { invoke } from "@tauri-apps/api/core";
import type { AppCleanupPolicy } from "../types";

export const IGNORED_APPS_KEY = "app.app_cleanup_policies";

export const normalizeAppPath = (path: string) => path.trim().replace(/\/+$/, "");

export const canonicalizeAppPaths = (paths: string[]) =>
  paths.length ? invoke<string[]>("canonicalize_app_paths", { paths }) : Promise.resolve([]);

const policyFields = ["id", "appName", "appPath", "action", "cleanupRules"] as const;

const hasPolicyFields = (value: unknown): value is AppCleanupPolicy => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const policy = value as Record<string, unknown>;
  return policyFields.every((key) => !(key in policy) || typeof policy[key] === "string")
    && (!("enabled" in policy) || typeof policy.enabled === "boolean")
    && (!("contentTypes" in policy) || (Array.isArray(policy.contentTypes) && policy.contentTypes.every((kind) => typeof kind === "string")));
};

export const parseAppPolicies = (raw?: string): AppCleanupPolicy[] => {
  if (!raw) return [];
  try {
    const policies: unknown = JSON.parse(raw);
    return Array.isArray(policies) && policies.every(hasPolicyFields) ? policies : [];
  } catch {
    return [];
  }
};

const isIgnored = (policy: AppCleanupPolicy) =>
  !!policy && policy.enabled !== false && policy.action?.toLowerCase() === "ignore";

export const listIgnoredApps = (policies: AppCleanupPolicy[]) =>
  policies.filter((policy) => isIgnored(policy) && (policy.appPath?.trim() || policy.appName?.trim()));

export const addIgnoredApp = (
  policies: AppCleanupPolicy[],
  { name, path }: { name: string; path: string },
  canonicalPaths: ReadonlyMap<string, string> = new Map(),
): AppCleanupPolicy[] => {
  const canonicalPath = (value: string) => normalizeAppPath(canonicalPaths.get(value) ?? value);
  const appPath = canonicalPath(path);
  if (!appPath) return policies;
  const matches = (policy: AppCleanupPolicy) =>
    canonicalPath(policy.appPath ?? "").toLowerCase() === appPath.toLowerCase();
  if (policies.some((policy) => matches(policy) && isIgnored(policy))) return policies;
  const existing = policies.findIndex(matches);
  if (existing >= 0) {
    const oldPath = policies[existing].appPath;
    return policies.map((policy, index) =>
      index === existing
        ? {
            ...policy,
            appPath: canonicalPath(oldPath) === normalizeAppPath(oldPath) ? oldPath : appPath,
            enabled: true,
            action: "ignore",
          }
        : policy);
  }
  return [...policies, {
    id: `ignored:${appPath.toLowerCase()}`,
    enabled: true,
    appName: name,
    appPath,
    action: "ignore",
    contentTypes: ["text", "code", "url", "rich_text"],
    cleanupRules: "",
  }];
};

export const removeIgnoredApp = (policies: AppCleanupPolicy[], target: AppCleanupPolicy): AppCleanupPolicy[] =>
  policies.flatMap((policy) => {
    if (!isIgnored(policy)) return [policy];
    const same = target.id
      ? policy.id === target.id
      : !policy.id && policy.appPath === target.appPath && policy.appName === target.appName;
    if (!same) return [policy];
    return policy.cleanupRules?.trim() ? [{ ...policy, action: "clean" }] : [];
  });
