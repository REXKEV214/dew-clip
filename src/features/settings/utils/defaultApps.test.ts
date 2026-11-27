import { describe, expect, it } from "vitest";
import { associationExtension, chosenApp, unchosenFallback } from "./defaultApps";

describe("default apps", () => {
  it("treats an empty or blank value as nothing chosen", () => {
    expect(chosenApp({ "app.image": "" }, "image")).toBe("");
    expect(chosenApp({ "app.image": "  " }, "image")).toBe("");
    expect(chosenApp({}, "image")).toBe("");
  });

  it("code without its own app follows the text app, like the backend", () => {
    expect(unchosenFallback({ "app.text": "C:\\npp.exe" }, "code")).toBe("text");
    expect(unchosenFallback({}, "code")).toBe("system");
    expect(unchosenFallback({ "app.text": "C:\\npp.exe" }, "image")).toBe("system");
  });

  it("asks for video, HTML files and web links through their own associations", () => {
    expect(associationExtension("video")).toBe(".mp4");
    expect(associationExtension("rich_text")).toBe(".html");
    expect(associationExtension("url")).toBe("url");
  });
});
