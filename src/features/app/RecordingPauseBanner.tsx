import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isTauriRuntime } from "../../shared/lib/tauriRuntime";
import type { Translate } from "../../shared/types";
import { useNow } from "../clipboard/useNow";

interface RecordingPauseStatus {
  paused: boolean;
  resumeAt: number | null;
}

export function useRecordingPause() {
  const [status, setStatus] = useState<RecordingPauseStatus | null>(null);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let active = true;
    let receivedChange = false;
    const subscription = listen<RecordingPauseStatus>("recording-pause-changed", ({ payload }) => {
      receivedChange = true;
      if (active) setStatus(payload);
    });
    void subscription.then(() => invoke<RecordingPauseStatus>("get_recording_pause_state")).then((value) => {
      if (active && !receivedChange) setStatus(value);
    }).catch(console.error);

    return () => {
      active = false;
      void subscription.then((off) => off()).catch(console.error);
    };
  }, []);

  return status;
}

export function recordingPauseMessage(t: Translate, resumeAt: number | null, now: number): string {
  if (resumeAt === null) return t("recording_paused");
  const remaining = resumeAt - now;
  if (remaining < 60_000) return t("recording_paused_under_min");
  return t("recording_paused_timed").replace("{min}", String(Math.ceil(remaining / 60_000)));
}

export default function RecordingPauseBanner({ t, resumeAt, onResume }: {
  t: Translate;
  resumeAt: number | null;
  onResume: () => void;
}) {
  const now = useNow();
  return (
    <div className="recording-pause-banner">
      <span>{recordingPauseMessage(t, resumeAt, now)}</span>
      <button type="button" onClick={onResume}>{t("recording_resume")}</button>
    </div>
  );
}
