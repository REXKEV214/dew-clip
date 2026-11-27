import { useEffect, useId, useMemo, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Search } from "lucide-react";
import type { InstalledAppOption } from "../types";
import { associationExtension } from "../utils/defaultApps";
import { normalizeAppPath } from "../utils/ignoredApps";

interface AppSelectorProps {
    type: string | null;
    installedApps: InstalledAppOption[];
    onSelect: (val: string) => void;
    t: (key: string) => string;
    mode?: "default-app" | "pick-app";
    excludePaths?: string[];
}

interface FlattenedAppItem {
    group: "recommended" | "all";
    app: InstalledAppOption;
}

const normalizeAppValue = (value: string) => normalizeAppPath(value).toLowerCase();

const AppSelector = ({
    type,
    installedApps,
    onSelect,
    mode = "default-app",
    excludePaths,
    t
}: AppSelectorProps) => {
    const [recommended, setRecommended] = useState<InstalledAppOption[]>([]);
    const [loading, setLoading] = useState(false);
    const [query, setQuery] = useState("");
    const [selectedIndex, setSelectedIndex] = useState(0);
    const inputRef = useRef<HTMLInputElement | null>(null);
    const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);
    const listboxId = useId();

    useEffect(() => {
        let cancelled = false;

        if (!type || mode === "pick-app") {
            setRecommended([]);
            setLoading(false);
            return () => {
                cancelled = true;
            };
        }

        const fetchRecommended = async () => {
            setLoading(true);
            try {
                const extension = associationExtension(type);
                let associatedApps: InstalledAppOption[] = [];
                if (extension) {
                    try {
                        const associated = await invoke<{ name: string; path: string }[]>(
                            "get_associated_apps",
                            { extension }
                        );
                        associatedApps = associated.map((app) => ({
                            label: app.name,
                            value: app.path
                        }));
                    } catch {
                        // Recommendations are optional; the complete installed-app list remains usable.
                    }
                }

                if (!cancelled) {
                    setRecommended(associatedApps);
                }
            } finally {
                if (!cancelled) {
                    setLoading(false);
                }
            }
        };

        void fetchRecommended();
        return () => {
            cancelled = true;
        };
    }, [type, mode]);

    useEffect(() => {
        setQuery("");
        setSelectedIndex(0);
    }, [type]);

    const otherApps = useMemo(() => {
        const recommendedValues = new Set(
            recommended.map((app) => app.value)
        );
        const excluded = new Set(excludePaths?.map(normalizeAppValue));
        return installedApps.filter(
            (app) => !recommendedValues.has(app.value) && !excluded.has(normalizeAppValue(app.value))
        );
    }, [installedApps, recommended, excludePaths]);

    const normalizedQuery = query.trim().toLowerCase();
    const filteredRecommended = useMemo(
        () => recommended.filter((app) => (
            !normalizedQuery
            || app.label.toLowerCase().includes(normalizedQuery)
            || app.value.toLowerCase().includes(normalizedQuery)
        )),
        [recommended, normalizedQuery]
    );
    const filteredOtherApps = useMemo(
        () => otherApps.filter((app) => (
            !normalizedQuery
            || app.label.toLowerCase().includes(normalizedQuery)
            || app.value.toLowerCase().includes(normalizedQuery)
        )),
        [otherApps, normalizedQuery]
    );

    const flattenedItems = useMemo<FlattenedAppItem[]>(
        () => [
            ...filteredRecommended.map((app) => ({ group: "recommended" as const, app })),
            ...filteredOtherApps.map((app) => ({ group: "all" as const, app }))
        ],
        [filteredRecommended, filteredOtherApps]
    );

    useEffect(() => {
        itemRefs.current.length = flattenedItems.length;
        if (flattenedItems.length === 0) {
            setSelectedIndex(0);
        } else {
            setSelectedIndex((current) => Math.min(current, flattenedItems.length - 1));
        }
    }, [flattenedItems]);

    useEffect(() => {
        itemRefs.current[selectedIndex]?.scrollIntoView({ block: "nearest" });
    }, [selectedIndex]);

    const handleSelect = (app: InstalledAppOption) => {
        onSelect(app.value);
    };

    const handleInputKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
        if (event.key === "ArrowDown") {
            event.preventDefault();
            setSelectedIndex((current) => (
                flattenedItems.length > 0
                    ? Math.min(current + 1, flattenedItems.length - 1)
                    : 0
            ));
            return;
        }
        if (event.key === "ArrowUp") {
            event.preventDefault();
            setSelectedIndex((current) => Math.max(current - 1, 0));
            return;
        }
        if (event.key === "Home" && flattenedItems.length > 0) {
            event.preventDefault();
            setSelectedIndex(0);
            return;
        }
        if (event.key === "End" && flattenedItems.length > 0) {
            event.preventDefault();
            setSelectedIndex(flattenedItems.length - 1);
            return;
        }
        if (event.key === "Enter") {
            const selected = flattenedItems[selectedIndex];
            if (selected) {
                event.preventDefault();
                handleSelect(selected.app);
            }
        }
    };

    const renderSection = (
        label: string,
        items: InstalledAppOption[],
        group: "recommended" | "all",
        startIndex: number
    ) => {
        if (items.length === 0) return null;

        return (
            <div className="app-selector-section" role="presentation">
                <div className="app-selector-section-label" role="presentation">
                    {label}
                </div>
                {items.map((app, offset) => {
                    const index = startIndex + offset;
                    const selected = index === selectedIndex;
                    const optionId = `${listboxId}-${group}-${index}`;
                    return (
                        <button
                            id={optionId}
                            key={`${group}-${app.value}`}
                            ref={(node) => {
                                itemRefs.current[index] = node;
                            }}
                            type="button"
                            role="option"
                            aria-selected={selected}
                            tabIndex={-1}
                            title={app.value}
                            className={`app-selector-option${selected ? " is-selected" : ""}`}
                            onMouseEnter={() => setSelectedIndex(index)}
                            onMouseDown={(event) => event.preventDefault()}
                            onClick={() => handleSelect(app)}
                        >
                            <span className="app-selector-option-label">{app.label}</span>
                        </button>
                    );
                })}
            </div>
        );
    };

    const activeOption = flattenedItems[selectedIndex];
    const activeOptionId = activeOption
        ? `${listboxId}-${activeOption.group}-${selectedIndex}`
        : undefined;

    return (
        <div className="app-selector">
            <div className="app-selector-search">
                <Search size={15} className="app-selector-search-icon" aria-hidden />
                <input
                    ref={inputRef}
                    autoFocus
                    data-autofocus
                    value={query}
                    role="combobox"
                    aria-autocomplete="list"
                    aria-expanded="true"
                    aria-controls={listboxId}
                    aria-activedescendant={activeOptionId}
                    aria-label={t("search_apps_placeholder")}
                    onFocus={() => {
                        invoke("focus_clipboard_window").catch(() => undefined);
                    }}
                    onChange={(event) => {
                        setQuery(event.target.value);
                        setSelectedIndex(0);
                    }}
                    onKeyDown={handleInputKeyDown}
                    placeholder={loading ? t("searching_apps") : t("search_apps_placeholder")}
                    spellCheck={false}
                />
            </div>

            <div id={listboxId} className="app-selector-list" role="listbox">
                {loading ? (
                    <div className="app-selector-status" role="status">
                        {t("searching_apps")}
                    </div>
                ) : flattenedItems.length === 0 ? (
                    <div className="app-selector-status" role="status">
                        {t("no_matching_apps")}
                    </div>
                ) : (
                    <>
                        {renderSection(t("system_recommended"), filteredRecommended, "recommended", 0)}
                        {renderSection(t("all_apps"), filteredOtherApps, "all", filteredRecommended.length)}
                    </>
                )}
            </div>
        </div>
    );
};

export default AppSelector;
