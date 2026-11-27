# Changelog

## 1.2.0 — 2026-11-28

### New
- **Background update downloads**: When the automatic update check (on by default in **Settings → General**) finds a new release, DewClip downloads the release DMG in the background to `~/Library/Caches/io.github.rexkev214.dewclip/updates/` and verifies its size and SHA-256 against the checksum GitHub publishes for the release file.
  - Only GitHub release download hosts (`github.com` release download URLs, redirected to `release-assets.githubusercontent.com`) are accepted, only verified files are offered, and requests send only `User-Agent: DewClip` with no device information.
  - Automatic downloads are skipped on metered connections (such as a Personal Hotspot) and in Low Data Mode, give up if stalled for 15 seconds or below about 100 KB/s, and retry an hour later. Manual **Check for Updates** starts the download right away, including on metered connections or in Low Data Mode.
  - Only one installer is kept at a time (older ones are deleted), and the cache is cleared when the update check is turned off, a version is skipped, or settings are reset.
- **In-app update installation**: When a verified installer is ready, a banner, a dot on the Settings button, and a menu bar item show that the update is downloaded and ready to install. Nothing is ever installed without asking, and user data (clipboard history and settings) is never touched.
  - **Install and Reopen** quits DewClip, replaces the app, and reopens the new version, keeping the old app as a backup until the new one opens and restoring it if anything fails.
  - When automatic replacement is not possible (the app folder is not writable, the app is run from elsewhere, or the new version is signed differently), DewClip quits and shows the verified installer in Finder so you can drag DewClip into Applications.
  - **Later** hides the banner until the next launch; the reminder returns on every launch of an older version while the downloaded update is waiting. If an install did not complete, DewClip notifies you once.
  - If a release has no suitable installer, DewClip falls back to linking to the release page with instructions for replacing the app manually.

### Changed
- **Fixed release code-signing certificate**: 1.2.0 is the first release signed with a fixed self-signed certificate (`"DewClip Release Signing"`, via `npm run tauri:build:release`, which fails without the maintainer's certificate; `npm run tauri:build` remains ad-hoc signed for building from source). The app is not signed with an Apple Developer ID and is not notarized. Tested on macOS 26 by replacing one build with another signed with the same certificate: Accessibility permission and auto-paste keep working across future updates without granting again.

### Upgrading
- **Manual install this time**: DewClip 1.1.0 can only notify about 1.2.0 and link to the release page, so download `DewClip_1.2.0.dmg`, quit DewClip, and drag `DewClip.app` into **Applications** to replace the old version. Starting with 1.2.0, future updates download in the app.
- **One-time Accessibility permission re-grant**: Upgrading from 1.0.x or 1.1.x to 1.2.0 resets Accessibility permission one last time because the signature changes from ad-hoc to the fixed `"DewClip Release Signing"` certificate. Re-grant it via the in-app prompt or **System Settings → Privacy & Security → Accessibility**. From 1.2.0 onward, updates signed with the same certificate keep the permission (tested on macOS 26).
- **First launch**: As before, a DMG downloaded in a browser still requires **System Settings → Privacy & Security → Open Anyway** on first launch.

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
