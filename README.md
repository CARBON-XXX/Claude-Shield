# Claude Shield

While Anthropic's Claude Code is an exceptionally powerful terminal-based AI coding assistant, versions **2.1.91–2.1.196** contained hidden environment-fingerprinting and restriction checks. To protect developer privacy sovereignty and restore client connectivity over local proxy relays, Claude Shield provides a zero-Cargo-dependency, ultra-lightweight automated privacy protection utility and real-time sentinel daemon written in Rust.

虽然 Anthropic 的 Claude Code 是一款极其强大的终端 AI 编程助手，但在 **2.1.91–2.1.196** 安装包中存在针对开发者本地环境的隐密审计与限制检测机制。为了确保开发者的隐私主权并解决由于本地网络限制、代理中转站导致的应用阻断，Claude Shield 提供一个采用 Rust 编写的、无 Cargo 外部依赖的轻量级自动化隐私防护工具与实时后台守护进程。

> **Note / 说明**: Anthropic removed the steganographic fingerprinting path in **2.1.197+**. On those builds `scan` correctly reports *No known mechanism detected*. Shield still patches older installs, enforces clean locales, audits proxy/base-url against the historical 147-domain list, and runs a pure-Rust local translation gateway.
>
> Anthropic 已在 **2.1.197+** 移除隐写指纹路径。对这些版本 `scan` 会正确显示“未检测到已知机制”。本工具仍可修补旧版安装、强制干净 locale、按历史 147 域名列表审计代理/Base URL，并运行纯 Rust 本地翻译网关。

---

## Claude's Detection Flow / Claude的检测流程

Affected Claude Code builds (2.1.91–2.1.196) executed the following auditing pipeline when `ANTHROPIC_BASE_URL` pointed away from `api.anthropic.com`:

受影响版本在 `ANTHROPIC_BASE_URL` 指向非官方端点时会执行以下检测流水线：

1. **Environment & Timezone Auditing / 环境变量与时区探测**
   - Reads local system timezones to check if they match restricted zones (`Asia/Shanghai` or `Asia/Urumqi`).
   - 本地读取系统时区，检测是否属于受限时区（`Asia/Shanghai` / `Asia/Urumqi`）。

2. **Gateway Hostname Verification / 中转站域名审查**
   - Evaluates configured custom API endpoints (`ANTHROPIC_BASE_URL`) against an obfuscated internal blacklist of **147 domains** (base64 + XOR `91`) plus AI-lab keywords.
   - 当检测到自定义 API 端点时，会比对内部硬编码的 **147 个域名**黑名单（base64 + XOR `91`）与 AI 实验室关键词列表。

3. **Prompt Steganography / 提示词隐写水印注入**
   - Replaces standard ASCII apostrophes (`\u0027`) with alternative Unicode equivalents (`\u2019`, `\u02BC`, `\u02B9`) within the "Today's date is…" system prompt line, and may flip date separators (`-` → `/`) for China timezones.
   - 在系统提示词 “Today's date is…” 中，将 ASCII 撇号替换为 Unicode 变体，并可能把日期分隔符从 `-` 改为 `/`。

---

## Features / 功能特性

- **Zero Cargo Dependencies / 零 Cargo 依赖**
  - Compiled into a single standalone Rust binary. No Node.js runtime required for the shield itself. Outbound TLS uses the system `curl` toolchain when the local translation gateway is enabled.
  - 编译为单个独立的 Rust 原生二进制；防护本体不依赖 Node.js。启用本地翻译网关时，出站 TLS 使用系统自带的 `curl`。

- **Surgical Byte Patching / 微创字节修补**
  - Same-length in-place overrides for timezone strings, the **full 147-domain ciphertext blob**, lab-keyword blob, and steganographic apostrophe returns — keeping file length intact.
  - 原位等长替换时区字符串、**完整 147 域名密文块**、实验室关键词密文块与隐写撇号返回值，保持文件长度不变。

- **Auto-Scrubbed Execution Wrapper / 自动脱敏运行外壳**
  - Shell aliases dynamically enforce clean locales (`TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8`) upon invocation.
  - 自动挂载终端别名，启动时强制隔离环境（`TZ=UTC LANG=en_US.UTF-8`）。

- **Terminal Proxy Auditing / 终端代理健康审计**
  - `claude-shield audit` inspects proxy variables and evaluates `ANTHROPIC_BASE_URL` against the decoded 147-domain + lab-keyword lists, with live TCP reachability checks.
  - `claude-shield audit` 审计代理变量，并将 Base URL 与 147 域名/实验室关键词列表比对，同时做 TCP 连通性检查。

- **Lightweight Sentinel Daemon / 实时监控守护进程**
  - Native OS file events: **kqueue EVFILT_VNODE** on macOS, **inotify** on Linux (metadata poll fallback elsewhere). Automatically reapplies hot-patches after package updates.
  - 原生系统文件事件：macOS 使用 **kqueue**，Linux 使用 **inotify**（其他平台回退元数据轮询）。`npm` 升级后自动热修补。

- **Pure-Rust Translation Gateway / 纯 Rust 翻译中转**
  - `start` launches a local HTTP gateway on `127.0.0.1:18989` (no Node.js) that can scrub Chinese prompt text before forwarding to Anthropic.
  - `start` 会在 `127.0.0.1:18989` 启动纯 Rust 本地网关（无需 Node.js），可在转发前清洗中文提示词。

---

## How to Use / 如何使用

### 1. Compile & Build / 编译与构建

```bash
git clone https://github.com/CACEB001/Claude-Shield.git
cd Claude-Shield
cargo build --release
```

Binary: `target/release/claude-shield`.

### 2. Register Commands / 注册全局快捷别名

```bash
./target/release/claude-shield install
source ~/.zshrc  # or ~/.bashrc
```

### 3. Commands Reference / 命令指南

#### Scan / 扫描
```bash
claude-shield scan
```

#### Patch / 修补
```bash
claude-shield patch
```

#### Restore / 还原
```bash
claude-shield restore
```

#### Audit / 代理与域名审计
```bash
claude-shield audit
```

#### Cloud Protection Configuration / 云端防护配置
```bash
claude-shield config
claude-shield config --proxy http://127.0.0.1:7890
claude-shield config --base-url https://your-custom-gateway.com
claude-shield config --enforce true
claude-shield config --clear
```

#### Sentinel Daemon / 守护进程
```bash
claude-shield start
claude-shield status
claude-shield logs
claude-shield stop
```

---

### Potential Leak Risks & Security Recommendations / 潜在泄露风险与安全建议

1. **DNS Leak Prevention / 防止 DNS 解析泄露**
   - Configure your proxy client for **Remote DNS** or global **TUN mode**.
   - 请确保代理客户端开启远程解析或 TUN 模式。

2. **TLS Fingerprints (JA3/JA4) / TLS 指纹**
   - The local gateway forwards via system `curl`. Route through a gateway that can obfuscate TLS client hellos if needed.
   - 本地网关通过系统 `curl` 出站；如需可走具备 TLS 混淆的节点。

3. **Global Telemetry Bypass / 旁路遥测**
   - Force `*.anthropic.com` through your secure tunnel with global proxy rules.
   - 用全局路由规则强制 `*.anthropic.com` 走安全隧道。

---

## License / 开源协议

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
