import { ArrowLeft, ListOrdered, Pin, Settings as SettingsIcon, Star, Trash2, X } from "lucide-react";
import { showNativeMenu } from "../../shared/lib/nativeMenu";
import { startWindowDrag } from "../../shared/lib/windowDrag";
import type { Translate, WindowPinMode } from "../../shared/types";

interface AppHeaderProps {
  t: Translate;
  showSettings: boolean;
  isWindowPinned: boolean;
  pinMode: WindowPinMode;
  favoritesOpen: boolean;
  sequentialActive: boolean;
  sequentialCount: number;
  sequentialHotkey: string;
  updateAvailable?: boolean;
  onBack: () => void;
  onToggleFavorites: () => void;
  onToggleSequential: () => void;
  onTogglePinned: () => void;
  onSelectPinMode: (mode: WindowPinMode) => void;
  onCloseWindow: (source: "button") => void;
  onClearHistory: () => void;
  onOpenSettings: () => void;
}

const AppHeader = ({
  t,
  showSettings,
  isWindowPinned,
  pinMode,
  favoritesOpen,
  sequentialActive,
  sequentialCount,
  sequentialHotkey,
  updateAvailable,
  onBack,
  onToggleFavorites,
  onToggleSequential,
  onTogglePinned,
  onSelectPinMode,
  onCloseWindow,
  onClearHistory,
  onOpenSettings,
}: AppHeaderProps) => {
  const alwaysOnTop = pinMode === "always_on_top";
  const pinTooltip = t(isWindowPinned
    ? (alwaysOnTop ? "unpin_window_top_tip" : "unpin_window_normal_tip")
    : (alwaysOnTop ? "pin_window_top_tip" : "pin_window_normal_tip"));
  const activeTooltip = sequentialHotkey
    ? (sequentialCount === 1 ? "sequential_btn_active_tip_one" : "sequential_btn_active_tip")
    : (sequentialCount === 1 ? "sequential_btn_active_no_key_one" : "sequential_btn_active_no_key");
  const sequentialTooltip = sequentialActive
    ? t(activeTooltip).replace("{count}", String(sequentialCount)).replace("{hotkey}", sequentialHotkey)
    : t("sequential_btn_tip");

  return <header className="app-header" onMouseDown={startWindowDrag}>
    <button
      type="button"
      className="window-close"
      title={t("close_window_tip")}
      aria-label={t("close_window")}
      onMouseDown={(event) => {
        // Clicking keeps the editor focused until the close request is resolved.
        event.stopPropagation();
        event.preventDefault();
      }}
      onClick={(event) => {
        event.stopPropagation();
        event.currentTarget.blur();
        onCloseWindow("button");
      }}
    >
      <X size={14} strokeWidth={2.2} aria-hidden="true" />
    </button>

    {showSettings ? (
      <button type="button" className="icon-button back-button" title={t("back")} aria-label={t("back")} onClick={onBack}>
        <ArrowLeft size={16} />
      </button>
    ) : (
      <>
        <span className="app-name">DewClip</span>
        <div className="header-actions">
          <button
            type="button"
            className={`favorites-button${favoritesOpen ? " is-active" : ""}`}
            title={t("favorites")}
            aria-label={t("favorites")}
            aria-pressed={favoritesOpen}
            onClick={onToggleFavorites}
          >
            <Star size={15} />
          </button>
          <button
            type="button"
            className={`icon-button sequential-button${sequentialActive ? " is-active" : ""}`}
            title={sequentialTooltip}
            aria-label={sequentialTooltip}
            aria-pressed={sequentialActive}
            onClick={onToggleSequential}
          >
            <ListOrdered size={15} />
            {sequentialActive && <span className="sequential-count" aria-hidden>{sequentialCount}</span>}
          </button>
          <button
            type="button"
            className={`icon-button${isWindowPinned ? (alwaysOnTop ? " is-active is-pin-top" : " is-active is-pin-normal") : ""}`}
            title={pinTooltip}
            aria-label={pinTooltip}
            aria-pressed={isWindowPinned}
            onClick={onTogglePinned}
            onContextMenu={(event) => {
              event.preventDefault();
              event.stopPropagation();
              const box = event.currentTarget.getBoundingClientRect();
              void showNativeMenu([
                { text: t("pin_mode_always_on_top"), checked: alwaysOnTop, action: () => onSelectPinMode("always_on_top") },
                { text: t("pin_mode_normal"), checked: !alwaysOnTop, action: () => onSelectPinMode("normal") },
              ], { x: box.left, y: box.bottom + 4 });
            }}
          >
            <Pin size={15} className="pin-icon" />
          </button>
          <button
            type="button"
            className="icon-button"
            title={t("clear_history")}
            aria-label={t("clear_history")}
            onClick={onClearHistory}
          >
            <Trash2 size={15} />
          </button>
          <button
            type="button"
            className="icon-button settings-button"
            title={t("settings")}
            aria-label={t("settings")}
            onClick={onOpenSettings}
          >
            <SettingsIcon size={15} />
            {updateAvailable && <span className="settings-update-dot" aria-hidden="true" />}
          </button>
        </div>
      </>
    )}
  </header>;
};

export default AppHeader;
