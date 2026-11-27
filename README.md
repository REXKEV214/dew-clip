# DewClip

DewClip 是一款 macOS 剪贴板工具，让你找回复制过的内容，并快速粘贴到当前应用。

## 下载与首次打开

从 [Releases](https://github.com/REXKEV214/dew-clip/releases) 下载 `DewClip_1.0.0.dmg`。

1. 打开 DMG，把 `DewClip.app` 拖入「应用程序」文件夹，然后从该文件夹打开 DewClip。
2. 如果 macOS 拦截首次打开，点击「完成」。
3. 打开「系统设置」，进入「隐私与安全性」。
4. 在「安全性」中找到 DewClip，点击「仍要打开」。
5. 在确认窗口再次选择「仍要打开」，然后用触控 ID 或密码验证。

## 功能

- **剪贴板历史**。自动记录文字、富文本、代码、网址、图片，以及从访达复制的视频和文件。重复复制会合并到同一条记录。有来源信息时，记录会显示来源应用的图标。
- **搜索与快捷粘贴**。默认按 `⌥Z` 打开面板。用搜索栏查找内容、来源应用或图片中的文字，用分类标签筛选。点击记录，或用 `↑`、`↓` 选中后按回车，即可粘贴。
- **批量操作与格式选择**。多选后可以批量删除，也可以调整粘贴顺序和连接格式。文字支持编辑，并可按原格式、纯文本或 Markdown 粘贴。粘贴后是否移到列表顶部可在设置中调整。
- **常用内容**。收藏常用文字、图片、视频和文件，可添加标题和标签，或按标签筛选。
- **窗口与复制提示**。可固定窗口并决定是否始终置顶。复制提示支持玻璃胶囊、黑色胶囊和黑色刘海，可调整停留时间或关闭。
- **图片识别与翻译**。用 Apple Vision 在本机提取图片中的中英文。中英翻译默认使用 Apple 翻译，也可选择 DeepL 或 OpenAI 兼容服务。

## 系统要求与权限

- 需要 macOS 14 或更新版本。安装包适用于搭载 Apple 芯片的 Mac。
- 系统液态玻璃效果和 Apple 翻译需要 macOS 26 或更新版本。较早版本的窗口使用毛玻璃效果。
- 要让 DewClip 把内容直接粘贴到其他应用，请按应用提示授予辅助功能权限。

## 数据与隐私

- 自动检查更新默认开启，每天向 GitHub Releases 查询新版本，不发送设备信息。可以在「设置 → 常用设置」关闭，或在设置页底部手动检查。

剪贴板记录和附件默认保存在 `~/Library/Application Support/io.github.rexkev214.dewclip`。可在设置里更改存储位置，或关闭持久化存储，让新记录只保留在当前会话。

DewClip 无需账号，历史记录保存在本机。使用第三方翻译时，手动提交的文字和用于测试连接的固定句子会发送给所选服务。翻译密钥加密保存在本机。macOS 管理 Apple 翻译的语言包。复制含网络图片的网页富文本时，DewClip 可能下载图片以保留原格式。

## 从源码构建

需要 Node.js、Rust 和包含 macOS 26 SDK 的 Xcode。先在项目根目录运行 `npm ci` 安装依赖。

启动开发版。开发服务器会持续运行，按 `Ctrl+C` 退出。

```bash
npm run tauri:dev
```

运行测试与代码检查。

```bash
npm test
npm run test:rust
npm run lint
```

构建发布版。

```bash
npm run tauri:build
```

开发版的数据目录为 `~/Library/Application Support/io.github.rexkev214.dewclip.dev`。构建生成的应用和 DMG 位于 `src-tauri/target/release/bundle/`。

## 许可证

本项目以 GPL-3.0 开源，Copyright © 2026 REXKEV214，详见 [LICENSE](LICENSE) 与 [NOTICE](NOTICE)。
