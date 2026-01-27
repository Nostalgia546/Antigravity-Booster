# Antigravity Booster

<h2 align="center">专为 Antigravity 打造的高颜值效能增强工具</h2>

<p align="center">
  <a href="#特性">特性</a> •
  <a href="#快速开始">快速开始</a> •
  <a href="#技术栈">技术栈</a> •
  <a href="#开源协议">开源协议</a>
</p>

---

Antigravity Booster 是一款专门为 Antigravity 用户设计的辅助增强工具。它旨在解决 Antigravity 在某些环境下不遵循系统代理的顽疾，同时集成了多账号管理与配额监控功能，为您提供更加流畅、高效的 AI 开发体验。

### 特性

-   **极高颜值**：采用现代简约设计语言，支持深色/浅色模式切换，丝滑的微动画效果。
-   **代理增强**：无需复杂的 TUN/增强模式，完美解决 Antigravity 不遵循系统代理的问题。
-   **多账号一键切换**：支持多个 Google 账号管理，随时随地一键切换活跃账号。
-   **余量实时监控**：集成 Quota 查询功能，实时图表展示历史用量趋势，配额消耗一目了然。
-   **安全隐私**：本地存储账号信息，支持数据加密导出备份，保护您的隐私安全。

### 界面预览

<p align="center">
  <img width="100%" alt="仪表盘" src="https://github.com/user-attachments/assets/595d1727-7408-4143-ab63-163a00b844dd" />
</p>

<div style="display: flex; gap: 10px;">
  <img width="100%" alt="账号管理" src="https://github.com/user-attachments/assets/b964ebc7-3f87-4738-aa45-2f886842cb2e" />
  <img width="100%" alt="登录" src="https://github.com/user-attachments/assets/c631d494-2535-416f-871f-d948791dd583" />
</div>

### 快速开始

#### 下载安装

如果您使用的是Windows系统，您可以直接从 Releases 页面下载最新的 `.exe` 安装包进行安装。

#### 基本使用

1.  **添加账号**：在“账号管理”页面点击“添加账号”，通过 Google OAuth 授权登录。
2.  **设置代理**：在设置页面或底部状态栏配置您的本地代理端口（如 127.0.0.1:7890）。
3.  **启动加速**：点击左下角的启用代理按钮，Antigravity Booster 将自动处理网络 hook，使其遵循系统代理。
4.  **监控配额**：在仪表盘随时查看各模型的剩余百分比及重置时间。

### 技术栈

-   **Frontend**: Vue 3 + TypeScript + Pinia
-   **Backend**: Rust + Tauri 2.0
-   **UI Performance**: Lucide Icons + Optimized CSS Transitions
-   **Hooking**: Custom C++ Proxy DLL integration

### 开源协议

本项目采用 [GNU General Public License v3.0 (GPL-3.0)](LICENSE) 协议进行分发。

---

如果这个项目对您有所帮助，请给一个 **Star**，这是对我最大的支持！
