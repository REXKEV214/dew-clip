import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { getSourceAppIcon, peekSourceAppIcon } from "../../../../shared/lib/sourceAppIcon";
import type { Translate } from "../../../../shared/types";
import type { AppCleanupPolicy } from "../../types";
import { Row } from "../controls";

const IgnoredAppIcon = ({ path }: { path?: string }) => {
  const [src, setSrc] = useState(() => peekSourceAppIcon(path) ?? null);
  useEffect(() => {
    let active = true;
    void getSourceAppIcon(path).then((icon) => {
      if (active) setSrc(icon);
    });
    return () => { active = false; };
  }, [path]);
  return src && <img src={src} width={18} height={18} alt="" draggable={false} />;
};

export const IgnoredApps = ({ t, apps, onAdd, onRemove }: {
  t: Translate;
  apps: AppCleanupPolicy[];
  onAdd: () => void;
  onRemove: (app: AppCleanupPolicy) => void;
}) => (
  <Row label={t("ignored_apps")} hint={t("ignored_apps_hint")} column>
    {apps.length === 0 ? (
      <span className="setting-subtitle">{t("ignored_apps_empty")}</span>
    ) : (
      <ul className="setting-details">
        {apps.map((app, index) => {
          const name = app.appName || app.appPath;
          const remove = t("ignored_apps_remove").replace("{app}", name);
          return (
            <li key={`${app.id ?? app.appPath}:${index}`}>
              <span className="setting-detail-label" title={app.appPath}>
                <IgnoredAppIcon key={app.appPath} path={app.appPath} />
                {name}
              </span>
              <button type="button" className="glass-button is-compact" title={remove} aria-label={remove} onClick={() => onRemove(app)}>
                <X size={13} />
              </button>
            </li>
          );
        })}
      </ul>
    )}
    <div className="inline-actions">
      <button type="button" className="glass-button is-compact" onClick={onAdd}>{t("ignored_apps_add")}</button>
    </div>
  </Row>
);
