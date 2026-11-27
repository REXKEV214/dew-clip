import { expect, it } from "vitest";
import type { AppCleanupPolicy } from "../types";
import { addIgnoredApp, listIgnoredApps, normalizeAppPath, parseAppPolicies, removeIgnoredApp } from "./ignoredApps";

it("adds and removes apps without adding a duplicate for a differently cased path", () => {
  const app = { name: "Notes", path: " /Applications/Notes.app/// " };
  expect(normalizeAppPath(app.path)).toBe("/Applications/Notes.app");
  const policies = addIgnoredApp([], app);
  expect(listIgnoredApps(policies)).toHaveLength(1);
  expect(addIgnoredApp(policies, { ...app, path: "/applications/notes.app" })).toBe(policies);
  expect(removeIgnoredApp(policies, policies[0])).toEqual([]);
});

it("restores a cleanup policy with its rules, content types, ID and other fields intact", () => {
  const clean = {
    id: "notes-policy",
    enabled: true,
    action: "clean",
    appName: "Notes",
    appPath: "/Applications/Notes.app/",
    cleanupRules: "confidential => ",
    contentTypes: ["text"],
    savedByAnOlderVersion: { keep: true },
  } as const;
  const previous = parseAppPolicies(JSON.stringify([clean]));
  const unrelated: AppCleanupPolicy = { ...previous[0], appName: "Browser", appPath: "/Applications/Browser.app" };
  const ignored = addIgnoredApp([...previous, unrelated], { name: "Notes", path: "/applications/notes.app" });
  expect(ignored).toEqual([{ ...clean, action: "ignore" }, unrelated]);
  expect(removeIgnoredApp(ignored, ignored[0])).toEqual([clean, unrelated]);
});

it("handles malformed values and recognises enabled legacy ignore policies", () => {
  expect(parseAppPolicies("[")).toEqual([]);
  expect(parseAppPolicies("{}")).toEqual([]);
  expect(parseAppPolicies('[{"appPath":false}]')).toEqual([]);
  const legacy = parseAppPolicies('[{"appName":"Notes","action":"IGNORE"},{"appName":"TextEdit","enabled":false,"action":"ignore"}]');
  expect(listIgnoredApps(legacy)).toEqual([legacy[0]]);
  expect(removeIgnoredApp(legacy, legacy[0])).toEqual([legacy[1]]);
});

it("stores the canonical bundle and recognises an existing alias", () => {
  const alias = "/Applications/Safari.app";
  const bundle = "/System/Cryptexes/App/System/Applications/Safari.app";
  const paths = new Map([[alias, bundle], [bundle, bundle]]);
  const policies = addIgnoredApp([], { name: "Safari", path: alias }, paths);
  expect(policies[0].appPath).toBe(bundle);

  const legacy = [{ ...policies[0], appPath: alias }];
  expect(addIgnoredApp(legacy, { name: "Safari", path: bundle }, paths)).toBe(legacy);
  const clean = [{ ...legacy[0], action: "clean" as const, cleanupRules: "secret => " }];
  const ignored = addIgnoredApp(clean, { name: "Safari", path: bundle }, paths);
  expect(ignored[0]).toEqual({ ...clean[0], action: "ignore", appPath: bundle });
});
