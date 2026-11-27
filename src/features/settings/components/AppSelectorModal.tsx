import { open } from "@tauri-apps/plugin-dialog";
import { X } from "lucide-react";
import AppSelector from "./AppSelector";
import AppModal from "../../../shared/components/AppModal";
import type { InstalledAppOption } from "../types";
import { withNativeDialog } from "../../../shared/lib/focus";
import { canonicalizeAppPaths } from "../utils/ignoredApps";

interface AppSelectorModalProps {
    show: string | null;
    installedApps: InstalledAppOption[];
    t: (key: string) => string;
    onClose: () => void;
    onSave: (type: string, val: string) => void;
    /** Label of the button that clears the chosen app; hidden when none is chosen. */
    resetLabel?: string;
    mode?: "default-app" | "pick-app";
    excludePaths?: string[];
}

const AppSelectorModal = ({ show, installedApps, t, onClose, onSave, resetLabel, mode, excludePaths }: AppSelectorModalProps) => (
    <AppModal
        open={Boolean(show)}
        onClose={onClose}
        overlayClassName="modal-overlay-app-selector"
        panelClassName="modal-panel glass-popover modal-panel-app-selector"
        ariaLabel={t("select_app_title")}
    >
        <div className="modal-panel-header">
            <h3 className="modal-title">{t("select_app_title")}</h3>
            <button type="button" className="modal-button icon-only" onClick={onClose} aria-label={t("cancel")}>
                <X size={15} />
            </button>
        </div>

        <div className="selector-container">
            <AppSelector
                key={show}
                type={show || ""}
                installedApps={installedApps}
                mode={mode}
                excludePaths={excludePaths}
                onSelect={(val) => {
                    if (show) onSave(show, val);
                    onClose();
                }}
                t={t}
            />
        </div>

        <div className="modal-panel-actions">
            {resetLabel && (
                <button
                    type="button"
                    className="modal-button app-selector-reset"
                    onClick={() => {
                        // An empty value is "nothing chosen": the backend opens the default.
                        if (show) onSave(show, "");
                        onClose();
                    }}
                >
                    {resetLabel}
                </button>
            )}
            <button
                type="button"
                className="modal-button"
                onClick={async () => {
                    try {
                        const selected = await withNativeDialog(() => open({
                            multiple: false,
                            defaultPath: "/Applications",
                            filters: [{
                                name: "Applications",
                                extensions: ["app"]
                            }]
                        }), "settings:choose-application");
                        if (typeof selected === "string" && show) {
                            const path = mode === "pick-app" ? selected : (await canonicalizeAppPaths([selected]))[0];
                            onSave(show, path);
                            onClose();
                        }
                    } catch (err) {
                        console.error(err);
                    }
                }}
            >
                {t("browse_file")}
            </button>
            <button type="button" className="modal-button" onClick={onClose}>
                {t("cancel")}
            </button>
        </div>
    </AppModal>
);

export default AppSelectorModal;
