import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { isTauriRuntime } from "../../shared/lib/tauriRuntime";

export interface Release {
  version: string;
  releaseUrl: string;
  notesPreview: string;
}

export interface UpdateSnapshot {
  currentVersion: string;
  checking: boolean;
  status: "idle" | "up_to_date" | "available" | "error";
  latest: Release | null;
  lastCheckedAt: number | null;
  skippedVersion: string | null;
  available: boolean;
  bannerDismissed: boolean;
  errorKind: "offline" | "timeout" | "rate_limited" | "failed" | null;
  retryAfterSecs: number | null;
}

export const useUpdateStatus = (canOpenDialog: () => boolean, onError: () => void) => {
  const [snapshot, setSnapshot] = useState<UpdateSnapshot | null>(null);
  const [dialogRelease, setDialogRelease] = useState<Release | null>(null);
  const [manualChecking, setManualChecking] = useState(false);
  const [manualResult, setManualResult] = useState<UpdateSnapshot | "failed" | null>(null);
  const manualInFlight = useRef(false);
  const canOpen = useRef(canOpenDialog);
  canOpen.current = canOpenDialog;

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let active = true;
    let events = 0;
    const status = listen<UpdateSnapshot>("update-status-changed", ({ payload }) => {
      if (!active) return;
      events++;
      setSnapshot(payload);
    });
    const open = listen("open-update-dialog", () => {
      if (!active || !canOpen.current()) return;
      void invoke<UpdateSnapshot>("get_update_state").then((snapshot) => {
        if (active && snapshot.available && canOpen.current()) setDialogRelease(snapshot.latest);
      }).catch(console.error);
    });
    void status.then(async () => {
      const revision = events;
      const initial = await invoke<UpdateSnapshot>("get_update_state");
      if (active && events === revision) setSnapshot(initial);
    }).catch(console.error);
    return () => {
      active = false;
      void status.then((off) => off()).catch(console.error);
      void open.then((off) => off()).catch(console.error);
    };
  }, []);

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
      setDialogRelease(null);
    } catch {
      onError();
    }
  };

  return {
    snapshot, dialogRelease, manualChecking, manualResult, check, dismiss, skip,
    openReleasePage: (url: string) => void openUrl(url).catch(onError),
    openDialog: (release: Release) => { if (canOpenDialog()) setDialogRelease(release); },
    later: (version: string) => {
      setDialogRelease(null);
      dismiss(version);
    },
  };
};
