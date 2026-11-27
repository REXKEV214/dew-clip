import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import AppHeader from "./features/app/AppHeader";
import SearchBox, { type SearchBoxHandle } from "./features/app/SearchBox";
import TypeTabs from "./features/app/TypeTabs";
import FixedPositionBanner from "./features/app/FixedPositionBanner";
import RecordingPauseBanner, { useRecordingPause } from "./features/app/RecordingPauseBanner";
import { useSettings } from "./features/app/useSettings";
import { useOverlays } from "./features/app/useOverlays";
import { useHistory } from "./features/app/useHistory";
import { useHistorySelection } from "./features/app/useHistorySelection";
import { useSidePanel } from "./features/app/useSidePanel";
import { usePasteActions } from "./features/app/usePasteActions";
import { hasOpenDialog, useKeyboard } from "./features/app/useKeyboard";
import { useWindowClose, type UnsavedChangesHandle } from "./features/app/useWindowClose";
import { useUpdateStatus } from "./features/app/useUpdateStatus";
import { UpdateBanner, UpdateDialog } from "./features/app/UpdateNotice";
import { useShortcutBadges } from "./features/app/useShortcutBadges";
import ClipboardCard from "./features/clipboard/ClipboardCard";
import ClipboardList, { type ClipboardListHandle } from "./features/clipboard/ClipboardList";
import FavoritesPanel, { type FavoritesPanelHandle } from "./features/favorites/FavoritesPanel";
import EditPanel from "./features/edit/EditPanel";
import TranslatePanel from "./features/translate/TranslatePanel";
import type { TranslationConfig } from "./features/translate/config";
import BatchPanel from "./features/batch/BatchPanel";
import QueuePanel from "./features/queue/QueuePanel";
import { useSequentialQueue } from "./features/queue/useSequentialQueue";
import {
  allVisibleSelected,
  applyEntryStates,
  batchDeleteMessage,
  deletionRequest,
  entryKey,
  refreshSelection,
  selectAllVisible,
  selectionCounts,
  toggleSelectAllVisible,
  type EntryKey,
  type EntryState,
} from "./features/batch/batchSelection";
import PreviewPanel from "./features/preview/PreviewPanel";
import { panelAfterPreview, useSpacePreview } from "./features/preview/useSpacePreview";
import SettingsPage from "./features/settings/components/SettingsPage";
import { useIgnoredApps } from "./features/settings/useIgnoredApps";
import PermissionOnboarding from "./shared/components/PermissionOnboarding";
import { ConfirmDialog, ToastContainer } from "./shared/components/Overlays";
import { useContextMenuBlock } from "./shared/hooks/useContextMenuBlock";
import { useFocusCoordinator } from "./shared/hooks/useFocusCoordinator";
import { useLiveSearch } from "./shared/hooks/useLiveSearch";
import { isTextEntry, type TypeFilter } from "./shared/lib/clipboardTypes";
import { showNativeMenu, type NativeMenuItem } from "./shared/lib/nativeMenu";
import { isTauriRuntime } from "./shared/lib/tauriRuntime";
import { getHotkeyDisplayTokens } from "./shared/lib/hotkeyDisplay";
import { formatError } from "./shared/lib/errorMessage";
import type { ClipboardEntry, SidePanelKind, WindowPinMode } from "./shared/types";

/** How much the CSS tint shows on systems with frosted glass only. */
const FROSTED_TINT_MAX = 0.72;
/** Longer than the side panel's animation, so its in-between widths aren't saved. */
const WINDOW_SIZE_SAVE_DELAY_MS = 500;
const SEARCH_DELAY_MS = 150;
const PIN_MODE_KEY = "app.window_pin_mode";

const normalizePinMode = (value?: string): WindowPinMode => value === "normal" ? "normal" : "always_on_top";

const App = () => {
  useFocusCoordinator();
  useContextMenuBlock();

  const settings = useSettings();
  const { t, flag, values, ready, save, remember } = settings;
  const { toasts, pushToast, confirmDialog, openConfirm, closeConfirm } = useOverlays(t);
  const ignoredApps = useIgnoredApps(settings, pushToast);
  const addIgnoredApp = ignoredApps.add;
  const recordingPause = useRecordingPause();
  const queue = useSequentialQueue();
  const sequentialActive = queue.kind !== "inactive";
  const sequentialActiveRef = useRef(sequentialActive);
  sequentialActiveRef.current = sequentialActive;
  const sequentialHotkey = getHotkeyDisplayTokens(values["app.sequential_hotkey"] ?? "Alt+V", { preferMacSymbols: true })
    .map(({ label }) => label).join("");

  const [showSettings, setShowSettings] = useState(false);
  const showSettingsRef = useRef(showSettings);
  showSettingsRef.current = showSettings;
  const liveSearch = useLiveSearch(SEARCH_DELAY_MS);
  const { draft, search, clear: resetSearch } = liveSearch;
  const [typeFilter, setTypeFilter] = useState<TypeFilter | null>(null);
  const [isWindowPinned, setIsWindowPinned] = useState(false);
  const pinMode = normalizePinMode(values[PIN_MODE_KEY]);
  const pinModeRef = useRef(pinMode);
  useEffect(() => { pinModeRef.current = pinMode; }, [pinMode]);
  const windowPinnedRef = useRef(isWindowPinned);
  const pinModeChange = useRef(Promise.resolve());
  const updateWindowPinned = useCallback((pinned: boolean) => {
    windowPinnedRef.current = pinned;
    setIsWindowPinned(pinned);
  }, []);
  const [settingFixedPosition, setSettingFixedPosition] = useState(false);
  const updateError = () => pushToast(t("update_failed"));
  const updates = useUpdateStatus(() => !settingFixedPosition && !hasOpenDialog(), updateError);
  const { dialogRelease, snapshot: updateStatus } = updates;
  const [editing, setEditing] = useState<ClipboardEntry | null>(null);
  const [batchItems, setBatchItems] = useState<ClipboardEntry[]>([]);
  const [batchDeleting, setBatchDeleting] = useState(false);
  const [previewEntry, setPreviewEntry] = useState<ClipboardEntry | null>(null);
  const [hasNativeGlass, setHasNativeGlass] = useState(true);
  const [translationConfig, setTranslationConfig] = useState<TranslationConfig | null>(null);
  const canTranslate = translationConfig?.available ?? false;

  const pushToastRef = useRef(pushToast);
  pushToastRef.current = pushToast;
  const tRef = useRef(t);
  tRef.current = t;

  const searchRef = useRef<SearchBoxHandle | null>(null);
  const listRef = useRef<ClipboardListHandle | null>(null);

  const { items, hasMore, loading, loaded, settled, loadMore, refetch, removeItems } = useHistory(
    search,
    typeFilter,
    ready
  );
  const { panel, open: openPanel, close: closePanel } = useSidePanel();
  const batchMode = panel.kind === "batch";
  const searchPending = draft.trim() !== search || !settled;
  const openQueuePanel = useCallback(() => {
    setShowSettings(false);
    setEditing(null);
    setBatchItems([]);
    setPreviewEntry(null);
    void openPanel("queue");
  }, [openPanel]);
  const itemsRef = useRef(items);
  itemsRef.current = items;
  const batchItemsRef = useRef(batchItems);
  batchItemsRef.current = batchItems;

  const { selectedIndex, setSelectedIndex, clearSelection } = useHistorySelection(
    items, search, typeFilter, Boolean(search) && !searchPending && !batchMode
  );

  const deleteAfterPaste = flag("app.delete_after_paste");
  const moveToTopAfterPaste = flag("app.move_to_top_after_paste", true);

  const clearSearch = useCallback(() => {
    resetSearch();
    clearSelection();
  }, [clearSelection, resetSearch]);

  const { paste, pasteImageText, copyOnly, failed } = usePasteActions({
    t,
    pushToast,
    deleteAfterPaste,
    moveToTopAfterPaste,
    afterPaste: clearSearch,
  });
  const sequentialFailed = useCallback((error: unknown) => {
    const message = String(error);
    const localized = ["sequential_inactive", "sequential_queue_changed", "sequential_release_modifiers"]
      .find((key) => message.includes(key));
    if (localized) pushToast(t(localized), 3000);
    else failed(error);
  }, [failed, pushToast, t]);

  const toggleBatch = useCallback((item: ClipboardEntry) => {
    setBatchItems((prev) =>
      prev.some((entry) => entry.id === item.id) ? prev.filter((entry) => entry.id !== item.id) : [...prev, item]
    );
  }, []);

  const addFavorite = useCallback(
    (item: ClipboardEntry) => {
      invoke("add_favorite_from_entry", { id: item.id })
        .then(() => pushToast(t("add_favorite_ok")))
        .catch((error) => {
          const text = String(error);
          if (text.includes("favorite_too_large")) pushToast(t("favorite_too_large"), 3000);
          else if (text.includes("file_missing")) pushToast(t("file_missing"), 3000);
          else pushToast(t("save_failed") + text, 3000);
        });
    },
    [pushToast, t]
  );

  // Batch mode has no keyboard pick: a highlighted card would look ticked.
  const startBatch = useCallback(
    (item: ClipboardEntry) => {
      setEditing(null);
      clearSelection();
      setBatchItems([item]);
      void openPanel("batch");
    },
    [clearSelection, openPanel]
  );

  // Cmd+A on the list.
  const startBatchSelectAll = useCallback(() => {
    setEditing(null);
    clearSelection();
    setBatchItems(selectAllVisible([], itemsRef.current));
    void openPanel("batch");
  }, [clearSelection, openPanel]);

  const toggleSelectAll = useCallback(() => {
    setBatchItems((prev) => toggleSelectAllVisible(prev, itemsRef.current));
  }, []);

  // The ticked entries follow the list (pin state), so the confirmation counts right.
  useEffect(() => {
    setBatchItems((prev) => refreshSelection(prev, items));
  }, [items]);

  // 分享: the system share menu, anchored to the card (right-click menu's last item).
  const shareEntry = useCallback(
    (item: ClipboardEntry) => {
      const card = document.getElementById(`clipboard-item-${item.id}`);
      const anchor = card?.getBoundingClientRect() ?? new DOMRect(0, 0, 0, 0);
      invoke("share_content", {
        id: item.id,
        content: item.content,
        contentType: item.content_type,
        anchor: { x: anchor.left, y: anchor.top, width: anchor.width, height: anchor.height },
      }).catch((error) => {
        const text = String(error);
        pushToast(text.includes("file_missing") ? t("file_missing") : t("share_failed") + text, 3000);
      });
    },
    [pushToast, t]
  );

  const showCardMenu = useCallback(
    (item: ClipboardEntry) => {
      // 打开: the entry in its default app (set per type in Settings), first for every type.
      const open: NativeMenuItem = {
        text: t("open"),
        action: () =>
          void invoke("open_content", { ...entryKey(item), content: item.content, contentType: item.content_type }).catch(
            (error) => {
              const text = String(error);
              pushToast(text.includes("file_missing") ? t("file_missing") : t("open_failed") + text, 3000);
            }
          ),
      };
      const batch: NativeMenuItem = { text: t("batch_paste"), action: () => startBatch(item) };
      const favorite = (key: string): NativeMenuItem => ({ text: t(key), action: () => addFavorite(item) });
      let menu: NativeMenuItem[];
      if (isTextEntry(item.content_type)) {
        menu = [
          {
            text: t("paste_format"),
            children: [
              { text: t("paste_format_original"), action: () => void paste(item, "original") },
              { text: t("paste_format_plain"), action: () => void paste(item, "plain") },
              { text: t("paste_format_markdown"), action: () => void paste(item, "markdown") },
            ],
          },
          favorite("add_favorite_text"),
          batch,
        ];
      } else if (item.content_type === "image") {
        // 粘贴为文字 lives on the card's own 文 button now.
        menu = [favorite("add_favorite_image"), batch];
      } else if (item.content_type === "video") {
        menu = [favorite("add_favorite_video"), batch];
      } else {
        menu = [favorite("add_favorite_file"), batch];
      }
      if (sequentialActive) {
        menu.push({
          text: t("sequential_add_to_queue"),
          action: () => void invoke("enqueue_sequential_entry", { entry: entryKey(item) }).catch(sequentialFailed),
        });
      }
      const share: NativeMenuItem = { text: t("share"), action: () => shareEntry(item) };
      const path = item.source_app_path?.trim();
      const ignoreApp: NativeMenuItem = {
        text: t("stop_recording_this_app"),
        enabled: Boolean(path),
        action: path ? () => addIgnoredApp(item.source_app, path, t("ignored_apps_added")) : undefined,
      };
      void showNativeMenu([open, ...menu, ignoreApp, share]);
    },
    [addFavorite, addIgnoredApp, paste, pushToast, sequentialActive, sequentialFailed, shareEntry, startBatch, t]
  );

  const pasteImageTextEntry = useCallback(
    (item: ClipboardEntry) => void pasteImageText(item),
    [pasteImageText]
  );

  const editEntry = useCallback(
    (item: ClipboardEntry) => {
      setEditing(item);
      void openPanel("edit");
    },
    [openPanel]
  );

  // 翻译 shares 编辑's entry slot: only one panel is open at a time.
  const translateEntry = useCallback(
    (item: ClipboardEntry) => {
      setEditing(item);
      void openPanel("translate");
    },
    [openPanel]
  );

  const togglePin = useCallback(
    (item: ClipboardEntry) => {
      invoke("toggle_clipboard_pin", { ...entryKey(item), isPinned: !item.is_pinned }).catch((error) =>
        pushToast(t("pin_failed") + String(error), 3000)
      );
    },
    [pushToast, t]
  );

  const deleteEntry = useCallback(
    (item: ClipboardEntry) => {
      const identity = entryKey(item);
      removeItems([identity]);
      invoke("delete_clipboard_entry", { ...identity }).catch((error) => {
        pushToast(t("delete_failed") + String(error), 3000);
        void refetch(true);
      });
    },
    [pushToast, refetch, removeItems, t]
  );

  const closeSidePanel = useCallback(() => {
    setEditing(null);
    setBatchItems([]);
    setPreviewEntry(null);
    void closePanel();
  }, [closePanel]);

  const previousQueueKind = useRef(queue.kind);
  useEffect(() => {
    if (previousQueueKind.current === queue.kind) return;
    previousQueueKind.current = queue.kind;
    if (!sequentialActive) {
      if (panel.kind === "queue") closeSidePanel();
      return;
    }
    if (isTauriRuntime()) {
      void getCurrentWindow().isVisible().then((visible) => {
        if (visible && sequentialActiveRef.current) openQueuePanel();
      }).catch(console.error);
    }
  }, [closeSidePanel, openQueuePanel, panel.kind, queue.kind, sequentialActive]);

  const stopSequential = () => void invoke("stop_sequential_session").catch(sequentialFailed);
  const startSequentialFromBatch = async (entries: EntryKey[]) => {
    await invoke("start_sequential_from_entries", { entries });
    if (windowPinnedRef.current) openQueuePanel();
  };

  const batchDeletingRef = useRef(false);
  const batchPastingRef = useRef(false);
  const deleteBatch = useCallback(
    async (frozen: EntryState[]) => {
      batchDeletingRef.current = true;
      setBatchDeleting(true);
      try {
        const count = await invoke<number>("delete_clipboard_entries", { items: frozen });
        removeItems(frozen);
        closeSidePanel();
        pushToast(t("batch_deleted").replace("{count}", String(count)));
      } catch (error) {
        if (String(error).includes("selection_changed")) {
          // Something ticked was deleted, replaced or (un)pinned meanwhile:
          // bring the ticks up to date and let the user confirm again.
          const states = await invoke<EntryState[]>("get_clipboard_entry_states", {
            ids: frozen.map((item) => item.id),
          }).catch(() => null);
          if (states) setBatchItems((prev) => applyEntryStates(prev, states));
          pushToast(t("batch_selection_changed"), 3000);
        } else {
          pushToast(t("delete_failed") + String(error), 3000);
        }
        void refetch(true);
      } finally {
        batchDeletingRef.current = false;
        setBatchDeleting(false);
      }
    },
    [closeSidePanel, pushToast, refetch, removeItems, t]
  );

  const requestBatchDelete = useCallback(() => {
    const selection = batchItemsRef.current;
    if (selection.length === 0 || batchDeletingRef.current) return;
    // Frozen now: what the dialog counts is exactly what gets deleted.
    const frozen = deletionRequest(selection);
    openConfirm({
      title: t("batch_delete_title"),
      message: batchDeleteMessage(t, selectionCounts(selection, itemsRef.current)),
      danger: true,
      confirmLabel: t("delete"),
      onConfirm: () => {
        closeConfirm();
        void deleteBatch(frozen);
      },
    });
  }, [closeConfirm, deleteBatch, openConfirm, t]);

  // A preview temporarily borrows the panel area from favorites or the queue.
  const panelUnderPreview = useRef<SidePanelKind | null | undefined>(undefined);

  const showPreview = useCallback(
    (entry: ClipboardEntry) => {
      panelUnderPreview.current = panel.kind;
      setPreviewEntry(entry);
      void openPanel("preview");
    },
    [openPanel, panel.kind]
  );

  const hidePreview = useCallback(() => {
    const previous = panelUnderPreview.current;
    if (previous === undefined) return;
    panelUnderPreview.current = undefined;
    setPreviewEntry(null);
    const restore = panelAfterPreview(previous, sequentialActiveRef.current);
    void (restore ? openPanel(restore) : closePanel());
  }, [closePanel, openPanel]);

  const toggleFavorites = useCallback(() => {
    if (panel.kind === "favorites") {
      closeSidePanel();
      return;
    }
    setEditing(null);
    setBatchItems([]);
    void openPanel("favorites");
  }, [closeSidePanel, openPanel, panel.kind]);

  const leaveSettings = useCallback(() => setShowSettings(false), []);

  // 常用's editor: Esc closes it before the panel, so unsaved text isn't lost.
  const [favoriteEditing, setFavoriteEditing] = useState(false);
  const favoritesRef = useRef<FavoritesPanelHandle>(null);
  const editRef = useRef<UnsavedChangesHandle>(null);
  const translateRef = useRef<UnsavedChangesHandle>(null);
  const typingInPanel =
    panel.kind === "edit" || panel.kind === "translate" || (panel.kind === "favorites" && favoriteEditing);
  const cancelPreview = useSpacePreview({
    enabled: !showSettings && !settingFixedPosition && !typingInPanel && !batchMode,
    items,
    selectedIndex,
    onOpen: showPreview,
    onClose: hidePreview,
  });
  const windowClose = useWindowClose({
    t,
    blocked: () => settingFixedPosition || confirmDialog.show || batchDeletingRef.current ||
      batchPastingRef.current || favoritesRef.current?.isSaving() === true,
    hasUnsavedChanges: () => Boolean(
      favoritesRef.current?.hasUnsavedChanges() ||
      editRef.current?.hasUnsavedChanges() ||
      translateRef.current?.hasUnsavedChanges()
    ),
    hideWindow: async () => {
      panelUnderPreview.current = undefined;
      cancelPreview.current();
      await invoke("escape_window_cmd");
    },
    openConfirm,
    closeConfirm,
  });

  const dismiss = useCallback((inSearch: boolean) => {
    if (settingFixedPosition) return true;
    if (panel.kind === "favorites" && (favoriteEditing || !inSearch || !(draft || search))) {
      if (!favoritesRef.current?.dismiss()) closeSidePanel();
      return true;
    }
    if (!showSettings && panel.kind !== "edit" && panel.kind !== "translate" && (draft || search)) {
      clearSearch();
      return true;
    }
    if (panel.kind) {
      closeSidePanel();
      return true;
    }
    if (showSettings) {
      leaveSettings();
      return true;
    }
    if (selectedIndex !== null) {
      clearSelection();
      return true;
    }
    return false;
  }, [
    clearSearch, clearSelection, closeSidePanel, draft, favoriteEditing, leaveSettings, panel.kind, search,
    selectedIndex, settingFixedPosition, showSettings,
  ]);

  // 编辑, 翻译 and 常用's editor take typing: the list's keys stay off while they are open.
  const listKeys = !showSettings && !batchMode && !typingInPanel;
  const showShortcutBadges = useShortcutBadges(listKeys && !settingFixedPosition && !confirmDialog.show);

  useKeyboard({
    items,
    enabled: listKeys,
    selectedIndex,
    setSelectedIndex,
    typeFilter,
    setTypeFilter,
    paste: (entry) => void paste(entry),
    pastePlain: (entry) => void paste(entry, "plain"),
    copy: (entry) => void copyOnly(entry),
    remove: deleteEntry,
    focusSearch: () => {
      if (!favoritesRef.current?.focusSearchIfWithin()) searchRef.current?.focus();
    },
    searchPending,
    flushSearch: () => liveSearch.flush(draft),
    isSearchComposing: (inSearch) => inSearch ? liveSearch.isComposing() : favoritesRef.current?.isSearchComposing() === true,
    dismiss,
    ...windowClose,
    settingFixedPosition,
    panel: panel.kind,
    blocked: showSettings || settingFixedPosition,
    batchCount: batchItems.length,
    canSelectAll: settled,
    startBatchSelectAll,
    toggleSelectAll,
    requestBatchDelete,
  });

  // The list keeps its own width while the side panel opens or shrinks away.
  const sidePanelResizingWindow = panel.listWidth !== null;
  useEffect(() => {
    if (!isTauriRuntime() || !ready) return;
    let timer: number | null = null;
    const onResize = () => {
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        if (sidePanelResizingWindow) return;
        const width = String(Math.round(window.innerWidth));
        const height = String(Math.round(window.innerHeight));
        invoke("save_setting", { key: "app.window_logical_width", value: width }).catch(console.error);
        invoke("save_setting", { key: "app.window_logical_height", value: height }).catch(console.error);
      }, WINDOW_SIZE_SAVE_DELAY_MS);
    };
    window.addEventListener("resize", onResize);
    return () => {
      if (timer !== null) window.clearTimeout(timer);
      window.removeEventListener("resize", onResize);
    };
  }, [ready, sidePanelResizingWindow]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const subscriptions = [
      // Each opening starts unfiltered: a filter is for digging one item out.
      listen("clipboard-shown", () => {
        setTypeFilter(null);
        clearSelection();
        listRef.current?.scrollToTop();
        if (sequentialActiveRef.current && !showSettingsRef.current) openQueuePanel();
      }),
      listen<boolean>("window-pinned-changed", ({ payload }) => updateWindowPinned(payload)),
      // The wake shortcut tried to hide a window pinned above other apps.
      listen("window-pinned-close-blocked", () => {
        if (pinModeRef.current === "always_on_top") pushToastRef.current(tRef.current("pinned_esc_hint"));
      }),
      listen("side-panel-collapsed", () => {
        panelUnderPreview.current = undefined;
        cancelPreview.current();
        setEditing(null);
        setBatchItems([]);
        setPreviewEntry(null);
      }),
    ];
    return () => {
      subscriptions.forEach((subscription) => void subscription.then((off) => off()));
    };
  }, [cancelPreview, clearSelection, openQueuePanel, updateWindowPinned]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    invoke<boolean>("supports_liquid_glass").then(setHasNativeGlass).catch(() => setHasNativeGlass(true));
    let alive = true;
    invoke<TranslationConfig>("get_translation_config").then((config) => {
      if (alive) setTranslationConfig(config);
    }).catch(() => {
      if (alive) setTranslationConfig(null);
    });
    return () => { alive = false; };
  }, []);

  const [glassLevelLive, setGlassLevelLive] = useState<number | null>(null);
  const glassLevel = glassLevelLive ?? Number(values["app.glass_level"] ?? 35);

  // macOS 26+ tints the native glass; before that this layer does.
  const shellStyle = useMemo(
    () =>
      ({
        "--frosted-tint": hasNativeGlass ? 0 : (glassLevel / 100) * FROSTED_TINT_MAX,
      }) as CSSProperties,
    [glassLevel, hasNativeGlass]
  );

  const showPinnedToast = (mode: WindowPinMode) =>
    pushToast(t(mode === "normal" ? "pinned_normal_hint" : "pinned_esc_hint"));

  const togglePinned = () => {
    const pinned = !windowPinnedRef.current;
    updateWindowPinned(pinned);
    invoke("set_window_pinned", { pinned }).catch(console.error);
    if (pinned) showPinnedToast(pinModeRef.current);
  };

  const selectPinMode = (mode: WindowPinMode, fromHeader: boolean) => {
    pinModeChange.current = pinModeChange.current.then(async () => {
      const changed = mode !== pinModeRef.current;
      if (changed) {
        await invoke("set_window_pin_mode", { mode });
        pinModeRef.current = mode;
        remember(PIN_MODE_KEY, mode);
      }
      if (!fromHeader) return;
      const shouldPin = !windowPinnedRef.current;
      if (shouldPin) {
        await invoke("set_window_pinned", { pinned: true });
        updateWindowPinned(true);
      }
      if (shouldPin || changed) showPinnedToast(mode);
    }).catch((error) => pushToast(t("save_failed") + String(error), 3000));
  };

  const clearHistory = () => {
    openConfirm({
      title: t("clear_confirm_title"),
      message: t("clear_confirm_message"),
      danger: true,
      confirmLabel: t("clear_confirm_ok"),
      onConfirm: () => {
        closeConfirm();
        invoke("clear_clipboard_history").catch((error) => pushToast(t("clear_failed") + String(error), 3000));
      },
    });
  };

  const resetSettings = () => {
    openConfirm({
      title: t("reset_defaults"),
      message: t("reset_confirm"),
      danger: true,
      confirmLabel: t("reset_ok"),
      onConfirm: () => {
        closeConfirm();
        invoke("reset_settings")
          .then(() => window.location.reload())
          .catch((error) => pushToast(t("reset_failed") + String(error), 3000));
      },
    });
  };

  const beginFixedPosition = () => {
    invoke("begin_fixed_position_setup").catch(console.error);
    setSettingFixedPosition(true);
  };

  const batchIds = useMemo(() => new Set(batchItems.map((item) => item.id)), [batchItems]);

  // No highlighted card in batch mode: it would look ticked.
  const selectedId = !batchMode && selectedIndex !== null ? items[selectedIndex]?.id : undefined;
  const allSelected = useMemo(() => allVisibleSelected(batchItems, items), [batchItems, items]);

  // Stable between keystrokes in the search box, so typing doesn't re-render the list.
  const renderItem = useCallback(
    (item: ClipboardEntry, index: number) => (
      <ClipboardCard
        item={item}
        t={t}
        selected={item.id === selectedId}
        batchMode={batchMode}
        batchChecked={batchIds.has(item.id)}
        shortcutNumber={showShortcutBadges && index < 9 ? index + 1 : undefined}
        onPaste={paste as (entry: ClipboardEntry) => void}
        onToggleBatch={toggleBatch}
        onContextMenu={showCardMenu}
        onEdit={editEntry}
        onTogglePin={togglePin}
        onDelete={deleteEntry}
        onPasteAsText={pasteImageTextEntry}
        onTranslate={canTranslate ? translateEntry : undefined}
        searchTerm={search}
      />
    ),
    [
      batchIds,
      batchMode,
      canTranslate,
      deleteEntry,
      editEntry,
      paste,
      pasteImageTextEntry,
      search,
      selectedId,
      showCardMenu,
      showShortcutBadges,
      t,
      toggleBatch,
      togglePin,
      translateEntry,
    ]
  );
  const loadMoreItems = useCallback(() => void loadMore(), [loadMore]);

  if (!ready) return <div className="app-shell" />;

  return (
    <div className="app-shell" style={shellStyle}>
      <div className="app-main" style={panel.listWidth ? { flex: `0 0 ${panel.listWidth}px` } : undefined}>
        <AppHeader
          t={t}
          showSettings={showSettings}
          isWindowPinned={isWindowPinned}
          pinMode={pinMode}
          favoritesOpen={panel.kind === "favorites"}
          sequentialActive={sequentialActive}
          sequentialCount={queue.items.length}
          sequentialHotkey={sequentialHotkey}
          updateAvailable={updateStatus?.available}
          onBack={leaveSettings}
          onToggleFavorites={toggleFavorites}
          onToggleSequential={sequentialActive ? stopSequential : () => void invoke("start_sequential_session").catch(sequentialFailed)}
          onTogglePinned={togglePinned}
          onSelectPinMode={(mode) => selectPinMode(mode, true)}
          onCloseWindow={windowClose.requestCloseWindow}
          onClearHistory={clearHistory}
          onOpenSettings={() => {
            closeSidePanel();
            setShowSettings(true);
          }}
        />

        {recordingPause?.paused && (
          <RecordingPauseBanner
            t={t}
            resumeAt={recordingPause.resumeAt}
            onResume={() => void invoke("resume_recording").catch((error) => pushToast(t("save_failed") + formatError(error, t)))}
          />
        )}
        <UpdateBanner t={t} status={updateStatus} showSettings={showSettings}
          onView={updates.openDialog} onDismiss={updates.dismiss} />

        {showSettings ? (
          <SettingsPage
            settings={settings}
            updates={updates}
            ignoredApps={ignoredApps}
            openConfirm={openConfirm}
            closeConfirm={closeConfirm}
            pinMode={pinMode}
            onSelectPinMode={(mode) => selectPinMode(mode, false)}
            translationConfig={translationConfig}
            onTranslationSaved={setTranslationConfig}
            pushToast={pushToast}
            onSetFixedPosition={beginFixedPosition}
            onResetSettings={resetSettings}
            onGlassLevel={setGlassLevelLive}
          />
        ) : (
          <>
            <div className="list-toolbar">
              <SearchBox
                ref={searchRef}
                t={t}
                value={draft}
                onChange={liveSearch.change}
                onSubmit={liveSearch.flush}
                onClear={clearSearch}
                onCompositionStart={liveSearch.startComposition}
                onCompositionEnd={liveSearch.endComposition}
                pending={searchPending}
              />
              <TypeTabs t={t} value={typeFilter} onChange={setTypeFilter} />
            </div>
            <div className="list-area">
              <ClipboardList
                ref={listRef}
                t={t}
                items={items}
                loaded={loaded}
                searching={search.length > 0 || typeFilter !== null}
                hasMore={hasMore}
                loading={loading}
                selectedIndex={selectedIndex}
                onLoadMore={loadMoreItems}
                renderItem={renderItem}
              />
            </div>
          </>
        )}
      </div>

      {panel.kind && (
        <aside className="side-panel">
          {panel.kind === "favorites" && (
            <FavoritesPanel
              ref={favoritesRef}
              t={t}
              pushToast={pushToast}
              onPasteFailed={failed}
              onEditingChange={setFavoriteEditing}
            />
          )}
          {panel.kind === "edit" && editing && (
            <EditPanel
              ref={editRef}
              key={`${editing.id}:${editing.created_at ?? 0}`}
              t={t}
              entry={editing}
              deleteAfterPaste={deleteAfterPaste}
              moveToTopAfterPaste={moveToTopAfterPaste}
              pushToast={pushToast}
              onPasteFailed={failed}
              onClose={closeSidePanel}
            />
          )}
          {panel.kind === "translate" && editing && translationConfig && (
            <TranslatePanel
              ref={translateRef}
              key={`${editing.id}:${editing.created_at ?? 0}`}
              t={t}
              entry={editing}
              config={translationConfig}
              deleteAfterPaste={deleteAfterPaste}
              moveToTopAfterPaste={moveToTopAfterPaste}
              pushToast={pushToast}
              onPasteFailed={failed}
              onClose={closeSidePanel}
            />
          )}
          {panel.kind === "preview" && previewEntry && (
            <PreviewPanel key={`${previewEntry.id}:${previewEntry.created_at ?? 0}`} t={t} entry={previewEntry} />
          )}
          {panel.kind === "queue" && sequentialActive && (
            <QueuePanel
              t={t}
              items={queue.items}
              hotkey={sequentialHotkey}
              onReorder={(queueIds) => void invoke("reorder_sequential_queue", { queueIds }).catch(sequentialFailed)}
              onRemove={(queueId) => void invoke("remove_sequential_entry", { queueId }).catch(sequentialFailed)}
              onClear={() => void invoke("clear_sequential_queue").catch(sequentialFailed)}
              onEnd={stopSequential}
              onPaste={() => void invoke("paste_next_step").catch(sequentialFailed)}
            />
          )}
          {panel.kind === "batch" && (
            <BatchPanel
              t={t}
              items={batchItems}
              allVisibleSelected={allSelected}
              canSelectAll={settled && items.length > 0}
              hasMore={hasMore}
              deleting={batchDeleting}
              onPastingChange={(pasting) => { batchPastingRef.current = pasting; }}
              onToggleSelectAll={toggleSelectAll}
              onDeleteSelected={requestBatchDelete}
              onReorder={setBatchItems}
              onRemove={toggleBatch}
              deleteAfterPaste={deleteAfterPaste}
              moveToTopAfterPaste={moveToTopAfterPaste}
              onPasteFailed={failed}
              onStartSequential={startSequentialFromBatch}
              onClose={closeSidePanel}
            />
          )}
        </aside>
      )}

      {settingFixedPosition && (
        <FixedPositionBanner
          t={t}
          onDone={(saved) => {
            setSettingFixedPosition(false);
            if (saved) pushToast(t("fixed_banner_saved"));
          }}
        />
      )}

      <ToastContainer toasts={toasts} />
      {dialogRelease && updateStatus && (
        <UpdateDialog
          t={t}
          currentVersion={updateStatus.currentVersion}
          release={dialogRelease}
          onLater={() => updates.later(dialogRelease.version)}
          onSkip={() => updates.skip(dialogRelease.version)}
          onOpenReleasePage={updates.openReleasePage}
        />
      )}
      <ConfirmDialog
        state={confirmDialog}
        confirmLabel={confirmDialog.confirmLabel ?? t("confirm")}
        cancelLabel={t("cancel")}
        onClose={closeConfirm}
      />
      <PermissionOnboarding
        ready={ready}
        t={t}
        onboardingDone={flag("app.onboarding_done")}
        onDone={() => save("app.onboarding_done", "true")}
        pushToast={pushToast}
      />
    </div>
  );
};

export default App;
