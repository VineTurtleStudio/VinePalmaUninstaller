<div align="center">

# VinePalmaUninstaller

**Linux 桌面应用的卸载与残留清理工具**

<a href="#-简体中文">简体中文</a> ·
<a href="#-繁體中文">繁體中文</a> ·
<a href="#-english">English</a> ·
<a href="#-日本語">日本語</a>

</div>

---

## 🇨🇳 简体中文

一个 Tauri 桌面前端 + Rust 后端的小工具，把 Linux 上散落在各个包管理器里的软件列到同一张清单里，卸载后顺手清掉残留的配置、缓存与图标。

### 支持的来源

| 来源 | 说明 |
|---|---|
| **APT / dpkg** | Debian / Ubuntu 系系统包（含 `apt` 数据库重载，重装后仍能识别） |
| **Flatpak** | user / system 两种安装域 |
| **Snap** | `snap list` 输出解析 |
| **AppImage** | 扫描常见目录下的 `.AppImage` 与桌面入口 |
| **Docker** | 镜像与容器 |
| **APM** | 原子化发行版（如 Vanilla OS）的包 |

### 环境要求

| 依赖 | 版本 |
|---|---|
| Node.js | ≥ 20 |
| Rust | ≥ 1.77 |
| 系统 | Linux（Debian / Ubuntu 系最佳，其他发行版按需安装对应包管理器） |

系统依赖：

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

### 快速开始

```bash
npm install
npm run tauri:dev     # 开发模式
```

打包：

```bash
npm run tauri build
```

> `npm run tauri:dev` 会先执行 `tauri:clean`（结束残留的 vite 与旧进程），避免开发时窗口连到旧实例。

### 使用流程

1. 启动后自动扫描，列表按来源分组（`default` 只列用户级软件，`advanced` 列全部）
2. 搜索框按名称过滤，点开可看详情与占用空间
3. 选中要卸载的项目 → 生成**卸载计划**（预演将执行的命令与要清理的路径）
4. 确认后执行；每一步的结果都会回显，失败不会中断后续步骤

### 设计约定

- **先评估、再执行**：`assess` 负责生成计划，`cleanup` 负责清理残留，中间有可读的确认环节
- **不静默删数据**：清理项会明确列出路径，用户确认后才动手
- **错误隔离**：某个包管理器不可用（未安装 snap 等）只记录错误，不影响其他来源

### 目录结构

```
src/                 前端（React + TS）
src-tauri/src/
  commands.rs        前端可调用的命令（扫描 / 预演 / 执行）
  runner.rs          外部命令执行与流式输出
  managers/          各包管理器适配
    apt.rs flatpak.rs snap.rs appimage.rs docker.rs apm.rs
    assess.rs        卸载计划生成
    cleanup.rs       残留清理
    icons.rs         图标提取与缓存
  models.rs          数据结构（Package / UninstallPlan / Result）
```

### 常见问题

- **扫描不到 Flatpak 应用**：确认 `flatpak list` 在当前用户下可用
- **卸载后图标还在**：清理阶段勾选「图标缓存」，或注销重新登录
- **权限不足**：系统级包的卸载需要 sudo，应用会提示而不会自行提权

## 🇹🇼 繁體中文

Tauri 桌面前端 + Rust 後端的小工具：把 Linux 上散落在各套件管理器裡的軟體列在同一份清單，卸載後順手清掉殘留的設定、快取與圖示。

### 支援的來源

| 來源 | 說明 |
|---|---|
| **APT / dpkg** | Debian / Ubuntu 系系統套件 |
| **Flatpak** | user / system 兩種安裝域 |
| **Snap** | 解析 `snap list` |
| **AppImage** | 掃描常見目錄的 `.AppImage` 與桌面入口 |
| **Docker** | 映像與容器 |
| **APM** | 原子化發行版（如 Vanilla OS） |

### 環境需求

| 依賴 | 版本 |
|---|---|
| Node.js | ≥ 20 |
| Rust | ≥ 1.77 |
| 系統 | Linux（Debian / Ubuntu 系最佳） |

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

### 快速開始

```bash
npm install
npm run tauri:dev
```

打包：`npm run tauri build`

### 使用流程

1. 啟動後自動掃描，清單依來源分組（`default` 僅使用者級，`advanced` 全部）
2. 搜尋框可依名稱過濾，點開可看詳情與佔用空間
3. 選取項目 → 生成**卸載計畫**（預演指令與要清理的路徑）
4. 確認後執行；每步結果都會回顯，失敗不中斷後續步驟

### 設計原則

- **先評估、再執行**：`assess` 產生計畫，`cleanup` 清理殘留
- **不靜默刪資料**：清理項目會明確列出路徑
- **錯誤隔離**：某個套件管理器不可用只記錄錯誤

## 🇬🇧 English

A Tauri front end with a Rust backend that lists software scattered across Linux package managers in one place, and cleans up leftover config, cache and icons after uninstalling.

### Supported sources

| Source | Notes |
|---|---|
| **APT / dpkg** | Debian / Ubuntu system packages (reloads the apt DB so reinstalls are detected) |
| **Flatpak** | user and system installations |
| **Snap** | parses `snap list` |
| **AppImage** | scans common directories for `.AppImage` files and desktop entries |
| **Docker** | images and containers |
| **APM** | atomic distributions such as Vanilla OS |

### Requirements

| Dependency | Version |
|---|---|
| Node.js | ≥ 20 |
| Rust | ≥ 1.77 |
| OS | Linux (Debian / Ubuntu best supported) |

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

### Quick start

```bash
npm install
npm run tauri:dev     # development
npm run tauri build   # release bundle
```

> `tauri:dev` first runs `tauri:clean` to kill leftover vite/old processes, so the window never attaches to a stale instance.

### Workflow

1. The app scans on launch and groups results by source (`default` = user-level only, `advanced` = everything)
2. Filter by name; expand a row for details and size
3. Select packages → an **uninstall plan** is generated (commands and paths are shown before anything runs)
4. Confirm to execute; results stream per step and a failing step does not abort the rest

### Design rules

- **Assess before acting** — `assess` builds the plan, `cleanup` removes leftovers
- **No silent deletion** — every cleanup path is listed for confirmation
- **Error isolation** — a missing package manager only records an error

### Layout

```
src/                    React front end
src-tauri/src/
  commands.rs           commands exposed to the UI (scan / plan / execute)
  runner.rs             external command execution with streamed output
  managers/             per-package-manager adapters
  models.rs             shared data structures
```

## 🇯🇵 日本語

Tauri のフロントエンドと Rust バックエンドで作られた、Linux のパッケージマネージャーを横断してソフトを一覧化し、アンインストール後に残る設定・キャッシュ・アイコンを掃除するツールです。

### 対応ソース

| ソース | 備考 |
|---|---|
| **APT / dpkg** | Debian / Ubuntu 系 |
| **Flatpak** | user / system 両方 |
| **Snap** | `snap list` を解析 |
| **AppImage** | よく使うディレクトリの `.AppImage` とデスクトップエントリ |
| **Docker** | イメージとコンテナ |
| **APM** | アトミック系ディストリビューション（Vanilla OS など） |

### 必要なもの

| 依存 | バージョン |
|---|---|
| Node.js | ≥ 20 |
| Rust | ≥ 1.77 |
| OS | Linux（Debian / Ubuntu 系が最適） |

### クイックスタート

```bash
npm install
npm run tauri:dev
npm run tauri build
```

### 使い方

1. 起動時に自動スキャン、ソース別にグループ表示（`default` はユーザーレベル、`advanced` は全体）
2. 名前で絞り込み、行を展開すると詳細とサイズ
3. 対象を選択 → **アンインストール計画**が生成（実行コマンドと削除パスを提示）
4. 確認して実行。各ステップの結果が表示され、失敗しても後続は継続

### 設計方針

- **評価してから実行** — `assess` が計画を作り、`cleanup` が残骸を消す
- **無言で消さない** — 削除対象のパスを必ず提示
- **エラーを隔離** — 使えないパッケージマネージャーはエラー記録のみ
---

## 许可 / License

本项目以 [MIT](./LICENSE) 许可证发布。

Licensed under the [MIT License](./LICENSE).
