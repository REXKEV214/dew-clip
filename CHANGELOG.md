# Changelog

## 1.1.0 — 2026-11-27

### New
- **Live search & file-name search**: Search now filters as you type (with IME composition support) and matches file and video names in addition to text, OCR text, and source app names.
- **Favorites search**: Search inside Favorites by title, tag, phrase text, or media file name.
- **Keyboard shortcuts**:
  - <kbd>⌘1</kbd>–<kbd>⌘9</kbd> to paste the top nine items, with number badges shown while holding <kbd>⌘</kbd>.
  - <kbd>⇧↩</kbd> to paste the selected item as plain text.
  - <kbd>⌘C</kbd> to copy the selected item without pasting.
  - <kbd>⌘W</kbd> to close and unpin the window, prompting before discarding unsaved changes.
- **Sequential paste**: Start a collecting session from the header or menu bar (new copies queue automatically), or start a one-off round from Batch mode. Reorder or remove queued items in the side panel and paste step by step with <kbd>⌥V</kbd> (customizable; active only while items are queued).
- **Pause recording from the menu bar**: Pause clipboard capture for 5 minutes, 15 minutes, 1 hour, or until resumed, with a dimmed menu bar icon and an in-app resume banner.
- **Ignored apps**: Exclude specific apps from clipboard recording in **Settings → Privacy**, or right-click any card and choose **Stop Recording This App**.
- **Remote image controls & hardening**: Toggle downloading remote images in copied rich text and set a per-image size limit (2–16 MB) in **Settings → Privacy**. Downloads are restricted to HTTPS URLs, with localhost and private/local network addresses blocked.
- **History retention, storage limits & usage**: Set how long history is kept (7 days, 30 days, 90 days, 1 year, or Forever) and an optional storage limit (500 MB–5 GB) that keeps pinned items, tagged items, and favorites intact. View a storage usage breakdown and clear the thumbnail cache in **Settings → Data**.
- **Dark mode**: Choose **System**, **Light**, or **Dark** appearance in **Settings → Personalization**, applied to both the main panel and the copy notification capsule.
- **Update check**: Automatic daily check against GitHub Releases (on by default; can be turned off in **Settings → General**) plus a manual **Check for Updates** button at the bottom of Settings. Links to the release page and never installs updates automatically.

### Improved
- Default wake shortcut for new installs is now <kbd>⌥Z</kbd> (existing installs keep their saved shortcut).
- Clearer window close button in the panel header.
- Default-app settings now query macOS for actual system default and associated applications.
- Menu bar menu and tooltip now follow language switches immediately.

### Fixed
- <kbd>⌘F</kbd> now reliably focuses the search box (in History, or in Favorites when focus is inside Favorites) instead of being blocked by the WebView.
- Narrowed Tauri window capabilities and production Content Security Policy (removed unused filesystem plugin permissions and limited URL opening to `https://`).
- Added a 5 MB size cap with log rotation (`dewclip.log` and one `.1` backup) so log files do not grow indefinitely.
- Pinned the `tauri-nspanel` dependency to a fixed revision and removed stray debug logging from clipboard paste operations.

### Upgrading
- **Re-grant Accessibility permission**: Because releases use ad-hoc signing, replacing `DewClip.app` causes macOS to drop the previously granted Accessibility permission. Re-enable DewClip via the in-app prompt, **Settings → General**, or **System Settings → Privacy & Security → Accessibility** so auto-paste continues working.
- **Automatic update check**: Enabled by default in 1.1.0, making at most one daily request to the GitHub Releases API (`User-Agent: DewClip`, no device identifiers). You can disable it anytime in **Settings → General**.

## 1.0.0 — 2026-11-27

Initial release.
