/** The app chosen in 打开方式 for a content type, or "" for none. */
export const chosenApp = (appSettings: Record<string, string>, type: string): string =>
  (appSettings[`app.${type}`] ?? "").trim();

/**
 * What opens a type with no app chosen. Code falls back to the text app when
 * one is set (services/content_handler.rs), otherwise the system default.
 */
export const unchosenFallback = (appSettings: Record<string, string>, type: string): "text" | "system" =>
  type === "code" && chosenApp(appSettings, "text") ? "text" : "system";

export const associationExtension = (type: string) => {
  switch (type) {
    case "image": return ".png";
    case "video": return ".mp4";
    case "text":
    case "code":
    case "file": return ".txt";
    case "url":
    case "link": return "url";
    case "html":
    case "rich_text": return ".html";
    case "rtf": return ".rtf";
    default: return "";
  }
};
