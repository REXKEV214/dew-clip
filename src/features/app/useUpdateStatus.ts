import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { isTauriRuntime } from "../../shared/lib/tauriRuntime";

export interface Release {
  version: string;
  releaseUrl: string;
  notesPreview: string;
  asset?: { size: number };
}

export type DownloadStatus =
  | { state: "downloading"; version: string; received: number; total: number }
  | { state: "deferred"; version: string; reason: "slow" | "restricted" | "space" | "storage" | "failed" };

export interface InstallPreview {
  requirementMatches: boolean;
  automatic: boolean;
}

export type InstallError = "missing" | "invalid" | "failed";
export const installError = (error: unknown): InstallError =>
  error === "missing" || error === "invalid" ? error : "failed";

export interface UpdateSnapshot {
  currentVersion: string;
  checking: boolean;
  status: "idle" | "up_to_date" | "available" | "error";
  latest: Release | null;
  lastCheckedAt: number | null;
  skippedVersion: string | null;
  available: boolean;
  noticeVersion: string | null;
  ready: Release | null;
  download: DownloadStatus | null;
  incompleteInstall: boolean;
  bannerDismissed: boolean;
  errorKind: "offline" | "timeout" | "rate_limited" | "failed" | null;
  retryAfterSecs: number | null;
}

export const noticedRelease = (snapshot: UpdateSnapshot | null): Release | null =>
  snapshot?.available ? (snapshot.ready?.version === snapshot.noticeVersion ? snapshot.ready : snapshot.latest) : null;

const inspect = (version: string) => invoke<InstallPreview>("inspect_update", { version });

export const useUpdateDialogTarget = (snapshot: UpdateSnapshot | null) => {
  const [selection, select] = useState<{ version: string; incomplete?: boolean } | null>(null);
  const version = selection?.version;
  const release = version && snapshot
    ? [snapshot.ready, snapshot.latest].find((release) => release?.version === version) ?? null
    : null;
  if (selection && snapshot && !release) select(null);
  return { release, select, incompleteVersion: release && selection?.incomplete ? release.version : null };
};

export const useUpdateStatus = (canOpenDialog: () => boolean, onError: () => void) => {
  const [snapshot, setSnapshot] = useState<UpdateSnapshot | null>(null);
  const { release: dialogRelease, select: selectDialog, incompleteVersion } = useUpdateDialogTarget(snapshot);
  const [inspected, setInspected] = useState<{ version: string; result: InstallPreview | InstallError } | null>(null);
  const [manualChecking, setManualChecking] = useState(false);
  const [manualResult, setManualResult] = useState<UpdateSnapshot | "failed" | null>(null);
  const manualInFlight = useRef(false);
  const canOpen = useRef(canOpenDialog);
  canOpen.current = canOpenDialog;

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let active = true;
    let events = 0;
    let openingIncomplete = false;
    const openIncomplete = async () => {
      if (!active || openingIncomplete || !canOpen.current()) return;
      openingIncomplete = true;
      try {
        const release = await invoke<Release | null>("take_incomplete_update");
        if (active && release) {
          selectDialog({ version: release.version, incomplete: true });
        }
      } finally {
        openingIncomplete = false;
      }
    };
    const status = listen<UpdateSnapshot>("update-status-changed", ({ payload }) => {
      if (!active) return;
      events++;
      setSnapshot(payload);
    });
    const shown = listen("clipboard-shown", () => void openIncomplete().catch(console.error));
    const open = listen("open-update-dialog", () => {
      if (!active || !canOpen.current()) return;
      const revision = events;
      void invoke<UpdateSnapshot>("get_update_state").then((current) => {
        if (!active || !canOpen.current()) return;
        const release = noticedRelease(current);
        if (events === revision) setSnapshot(current);
        if (release) selectDialog({ version: release.version });
      }).catch(console.error);
    });
    void status.then(async () => {
      const revision = events;
      const initial = await invoke<UpdateSnapshot>("get_update_state");
      if (!active) return;
      if (events === revision) setSnapshot(initial);
      if (initial.incompleteInstall) {
        await shown;
        if (active && await getCurrentWindow().isVisible()) await openIncomplete();
      }
    }).catch(console.error);
    return () => {
      active = false;
      void status.then((off) => off()).catch(console.error);
      void open.then((off) => off()).catch(console.error);
      void shown.then((off) => off()).catch(console.error);
    };
  }, [selectDialog]);

  const readyVersion = dialogRelease?.version === snapshot?.ready?.version ? snapshot?.ready?.version : null;
  useEffect(() => {
    if (!readyVersion) return;
    let active = true;
    void inspect(readyVersion).then((result) => {
      if (active) setInspected({ version: readyVersion, result });
    }, (error: unknown) => {
      if (active) setInspected({ version: readyVersion, result: installError(error) });
    });
    return () => {
      active = false;
      setInspected(null);
    };
  }, [readyVersion]);

  const check = async () => {
    if (manualInFlight.current) return;
    manualInFlight.current = true;
    setManualChecking(true);
    setManualResult(null);
    try {
      setManualResult(await invoke<UpdateSnapshot>("check_for_updates_now"));
    } catch {
      setManualResult("failed");
    } finally {
      manualInFlight.current = false;
      setManualChecking(false);
    }
  };

  const dismiss = (version: string) =>
    void invoke("dismiss_update_banner", { version }).catch(onError);

  const skip = async (version: string) => {
    try {
      await invoke("skip_update_version", { version });
      selectDialog(null);
    } catch {
      onError();
    }
  };

  return {
    snapshot, dialogRelease, incompleteVersion, manualChecking, manualResult, check, dismiss, skip,
    installPreview: readyVersion && inspected?.version === readyVersion ? inspected.result : null,
    install: async (version: string, confirmedAutomatic: boolean | null) => {
      const revised = await invoke<InstallPreview | null>("install_update", { version, confirmedAutomatic });
      if (revised) setInspected({ version, result: revised });
      return revised;
    },
    download: (version: string) => void invoke("download_update_now", { version }).catch(onError),
    openReleasePage: (url: string) => void openUrl(url).catch(onError),
    openDialog: (release: Release) => { if (canOpenDialog()) selectDialog({ version: release.version }); },
    later: (version: string) => {
      selectDialog(null);
      dismiss(version);
    },
  };
};
