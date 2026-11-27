import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { EntryKey } from "../../shared/lib/clipboardTypes";
import { isTauriRuntime } from "../../shared/lib/tauriRuntime";

export interface SequentialItem {
  queueId: number;
  identity: EntryKey;
  contentType: string;
  preview: string;
  mediaPath: string | null;
  timestamp: number;
}

export interface SequentialQueue {
  kind: "inactive" | "recording" | "one_shot";
  items: SequentialItem[];
  pastedInRound: number;
  roundTotal: number;
}

const INACTIVE: SequentialQueue = { kind: "inactive", items: [], pastedInRound: 0, roundTotal: 0 };

export function useSequentialQueue() {
  const [queue, setQueue] = useState(INACTIVE);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let active = true;
    let receivedChange = false;
    const subscription = listen<SequentialQueue>("sequential-queue-changed", ({ payload }) => {
      receivedChange = true;
      if (active) setQueue(payload);
    });
    void subscription.then(() => invoke<SequentialQueue>("get_sequential_queue")).then((value) => {
      if (active && !receivedChange) setQueue(value);
    }).catch(console.error);
    return () => {
      active = false;
      void subscription.then((off) => off()).catch(console.error);
    };
  }, []);

  return queue;
}
