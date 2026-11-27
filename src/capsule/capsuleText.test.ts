import { describe, expect, it } from "vitest";
import { capsuleLocale, capsuleShowsCheck, capsuleSubtitle, capsuleTitle, knownAppName } from "./capsuleText";

describe("capsule text", () => {
  it("names what was copied", () => {
    expect(capsuleTitle({ kind: "image", fileCount: 0 }, "zh")).toBe("已复制图片");
    expect(capsuleTitle({ kind: "url", fileCount: 0 }, "en")).toBe("Link copied");
    expect(capsuleTitle({ kind: "file", fileCount: 3 }, "zh")).toBe("已复制 3 个文件");
    expect(capsuleTitle({ kind: "file", fileCount: 1 }, "en")).toBe("File copied");
    expect(capsuleTitle({ kind: "something-new", fileCount: 0 }, "en")).toBe("Copied");
  });

  it("shows the preview, else where it came from", () => {
    expect(capsuleSubtitle({ kind: "text", preview: " hello ", appName: "Safari" }, "en")).toBe("hello");
    expect(capsuleSubtitle({ kind: "image", preview: "", appName: "Safari" }, "zh")).toBe("来自 Safari");
    expect(capsuleSubtitle({ kind: "image", preview: "", appName: "macOS App" }, "zh")).toBe("");
  });

  it("drops the capture's placeholder app names", () => {
    expect(knownAppName("Unknown")).toBe("");
    expect(knownAppName(" Notes ")).toBe("Notes");
  });

  it("uses the saved language when there is one", () => {
    expect(capsuleLocale("zh")).toBe("zh");
    expect(capsuleLocale("en")).toBe("en");
  });

  it("shows the pasted count and next preview", () => {
    const payload = {
      kind: "text", fileCount: 0, preview: "Next address", appName: "",
      sequential: { kind: "pasted", current: 2, total: 5 } as const,
    };
    expect(capsuleTitle(payload, "en")).toBe("Pasted 2/5");
    expect(capsuleSubtitle(payload, "en")).toBe("Next: Next address");
  });

  it("asks for the saved shortcut after skipping a deleted item", () => {
    const payload = {
      kind: "", fileCount: 0, preview: "", appName: "",
      sequential: { kind: "skipped", remaining: 1, hotkey: "Ctrl+Alt+P", roundEnded: false } as const,
    };
    expect(capsuleTitle(payload, "en")).toBe("Skipped a deleted item");
    expect(capsuleSubtitle(payload, "en")).toBe("Press ⌃⌥P again");
    expect(capsuleTitle(payload, "zh")).toBe("已跳过一项（记录已删除）");
    expect(capsuleSubtitle(payload, "zh")).toBe("再按一次 ⌃⌥P");
  });

  it("reports that the queue is empty or the one-shot has ended when the last item is skipped", () => {
    const blank = { kind: "", fileCount: 0, preview: "", appName: "" };
    const recording = { ...blank, sequential: { kind: "skipped", remaining: 0, hotkey: null, roundEnded: false } as const };
    const oneShot = { ...blank, sequential: { ...recording.sequential, roundEnded: true } };
    expect(capsuleTitle(recording, "en")).toBe("Skipped a deleted item");
    expect(capsuleSubtitle(recording, "en")).toBe("Queue is empty");
    expect(capsuleSubtitle(recording, "zh")).toBe("队列已空");
    expect(capsuleSubtitle(oneShot, "en")).toBe("Sequential Paste has ended.");
    expect(capsuleSubtitle(oneShot, "zh")).toBe("顺序粘贴已结束。");
  });

  it("shows the remaining count without a retry instruction when there is no registered shortcut", () => {
    const payload = {
      kind: "", preview: "", appName: "",
      sequential: { kind: "skipped", remaining: 1, hotkey: null, roundEnded: false } as const,
    };
    expect(capsuleSubtitle(payload, "en")).toBe("1 queued");
  });

  it("omits the success check on failed and skipped pastes", () => {
    expect(capsuleShowsCheck({ sequential: { kind: "failed", reason: "paste" } })).toBe(false);
    expect(capsuleShowsCheck({ sequential: { kind: "empty" } })).toBe(false);
    expect(capsuleShowsCheck({ sequential: { kind: "skipped", remaining: 1, hotkey: null, roundEnded: false } })).toBe(false);
    expect(capsuleShowsCheck({ sequential: { kind: "pasted", current: 1, total: 2 } })).toBe(true);
    expect(capsuleShowsCheck({})).toBe(true);
  });

  it("omits the next-item subtitle when the item type is unknown and has no preview", () => {
    const payload = { kind: "unknown", preview: "", appName: "", sequential: { kind: "pasted", current: 1, total: 2 } as const };
    expect(capsuleSubtitle(payload, "en")).toBe("");
    expect(capsuleSubtitle({ ...payload, kind: "file" }, "en")).toBe("Next: Files");
  });

  it("distinguishes a finished collecting round from a finished one-shot", () => {
    const blank = { kind: "", fileCount: 0, preview: "", appName: "" };
    const recording = { ...blank, sequential: { kind: "round_done", total: 3 } as const };
    const oneShot = { ...blank, sequential: { kind: "done", total: 3 } as const };
    expect(capsuleTitle(recording, "en")).toBe("Round complete (3)");
    expect(capsuleSubtitle(recording, "en")).toBe("Copy again to start a new round");
    expect(capsuleTitle(oneShot, "en")).toBe("Sequential Paste Done");
    expect(capsuleSubtitle(oneShot, "en")).toBe("Pasted 3 items");
    expect(capsuleSubtitle({ ...oneShot, sequential: { kind: "done", total: 1 } }, "en")).toBe("Pasted 1 item");
  });
});
