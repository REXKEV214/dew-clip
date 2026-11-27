import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SettingsStore } from "../../../app/useSettings";
import type { ConfirmDialogState } from "../../../../shared/types";
import { invalidateThumbnailCache } from "../../../../shared/hooks/useImageThumbnail";
import { RETENTION_OPTIONS, SIZE_OPTIONS, formatStorageBytes, storageCleanupMessage, tightensStorageRule, type StorageUsage } from "../../utils/storageSettings";
import { Row, Segmented } from "../controls";

const RETENTION_KEY = "app.history_retention_days";
const SIZE_KEY = "app.storage_limit_mb";

export const StorageSettings = ({ settings, pushToast, openConfirm, closeConfirm }: {
  settings: SettingsStore;
  pushToast: (message: string) => void;
  openConfirm: (options: Omit<ConfirmDialogState, "show">) => void;
  closeConfirm: () => void;
}) => {
  const { t, language, flag, remember, values } = settings;
  const [busy, setBusy] = useState(false);
  const [usage, setUsage] = useState<StorageUsage | null>(null);
  const [usageFailed, setUsageFailed] = useState(false);
  const [revision, setRevision] = useState(0);
  const refreshUsage = () => setRevision((current) => current + 1);
  const persistent = flag("app.persistent", true);

  useEffect(() => {
    let active = true;
    setUsageFailed(false);
    invoke<StorageUsage>("get_storage_usage").then((result) => {
      if (active) setUsage(result);
    }).catch(() => {
      if (active) setUsageFailed(true);
    });
    return () => { active = false; };
  }, [revision]);

  const applyRule = (key: string, value: string) => {
    const previous = values[key] ?? "0";
    if (previous === value || busy) return;
    const cleanup = tightensStorageRule(previous, value);
    const apply = async () => {
      setBusy(true);
      try {
        if (cleanup) {
          const count = await invoke<number>("run_storage_cleanup_now", { change: { key, value } });
          pushToast(storageCleanupMessage(count, t));
        } else {
          await invoke("save_setting", { key, value });
        }
        remember(key, value);
      } catch (error) {
        pushToast(`${t("storage_setting_failed")} ${String(error)}`);
        if (cleanup) {
          // The rule may have been saved before a cleanup error.
          await invoke<Record<string, string>>("get_settings")
            .then((current) => remember(key, current[key] ?? "0"))
            .catch(console.error);
        }
      } finally {
        setBusy(false);
        refreshUsage();
      }
    };
    if (cleanup) {
      openConfirm({
        title: t("storage_cleanup_confirm_title"),
        message: t("storage_cleanup_confirm_message"),
        danger: true,
        onConfirm: () => {
          closeConfirm();
          void apply();
        },
      });
    } else {
      void apply();
    }
  };

  const clearCache = async () => {
    setBusy(true);
    try {
      await invoke("clear_thumbnail_cache");
      pushToast(t("storage_cache_cleared"));
    } catch (error) {
      pushToast(`${t("storage_cache_clear_failed")} ${String(error)}`);
    } finally {
      invalidateThumbnailCache();
      setBusy(false);
      refreshUsage();
    }
  };

  const format = (bytes: number) => formatStorageBytes(bytes, language === "zh" ? "zh-CN" : "en");
  const clearButton = (
    <button type="button" className="glass-button is-compact" disabled={busy} onClick={() => void clearCache()}>
      {t("storage_clear_cache")}
    </button>
  );

  return (
    <>
      {persistent ? (
        <>
          <Row label={t("history_retention")} hint={t("history_retention_hint")} column>
            <Segmented
              value={values[RETENTION_KEY] ?? "0"}
              options={RETENTION_OPTIONS.map(([value, label]) => ({ value, label: t(label) }))}
              onChange={(value) => applyRule(RETENTION_KEY, value)}
              disabled={busy}
            />
          </Row>
          <Row label={t("storage_limit")} hint={t("storage_limit_hint")} column>
            <Segmented
              value={values[SIZE_KEY] ?? "0"}
              options={SIZE_OPTIONS.map(([value, label]) => ({ value, label: t(label) }))}
              onChange={(value) => applyRule(SIZE_KEY, value)}
              disabled={busy}
            />
          </Row>
        </>
      ) : (
        <div className="setting-row setting-subtitle">{t("storage_session_only")}</div>
      )}
      <Row label={t("storage_usage")} column>
        {!usageFailed && usage?.protectedOverLimit && (
          <p className="setting-subtitle" role="status">{t("storage_protected_over_limit")}</p>
        )}
        {usageFailed ? (
          <span className="setting-subtitle">{t("storage_usage_failed")}</span>
        ) : !usage ? (
          <span className="setting-subtitle" role="status">{t("storage_usage_loading")}</span>
        ) : (
          <dl className="setting-details">
            <div><dt>{t("storage_usage_total")}</dt><dd>{format(usage.totalBytes)}</dd></div>
            <div><dt>{t("storage_usage_db")}</dt><dd>{format(usage.databaseBytes)}</dd></div>
            <div><dt>{t("storage_usage_attachments")}</dt><dd>{format(usage.historyAttachmentsBytes)}</dd></div>
            <div><dt>{t("storage_usage_favorites")}</dt><dd>{format(usage.favoritesMediaBytes)}</dd></div>
            <div>
              <dt>{t("storage_usage_thumbnails")}</dt>
              <dd className="inline-actions">{format(usage.thumbnailCacheBytes)} {clearButton}</dd>
            </div>
          </dl>
        )}
        {(!usage || usageFailed) && <div className="inline-actions">{clearButton}</div>}
      </Row>
    </>
  );
};
