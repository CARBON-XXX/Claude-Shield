# Claude Shield

While Anthropic's Claude Code is an exceptionally powerful terminal-based AI coding assistant, we discovered several hidden environment-fingerprinting and restriction checks in its latest installation packages. To protect developer privacy sovereignty and restore client connectivity over local proxy relays, we introduced Claude Shield — a zero-dependency, ultra-lightweight automated privacy protection utility and real-time sentinel daemon written in Rust.

虽然 Anthropic 的 Claude Code 是一款极其强大的终端 AI 编程助手，但我们在其最新的安装包中发现了多项针对开发者本地开发环境的隐密审计与限制检测机制。为了确保开发者的隐私主权并解决由于本地网络限制、代理中转站导致的应用阻断，我们推出了 Claude Shield —— 一个采用 Rust 编写的、无任何外部依赖的轻量级自动化隐私防护工具与实时后台守护进程。

---

## Claude's Detection Flow / Claude的检测流程

The Claude client executes the following auditing pipeline during execution to profile local environments:

Claude 客户端在启动和运行期间会执行以下检测流水线，来对本地开发环境进行画像：

1. **Environment & Timezone Auditing / 环境变量与时区探测**
   - Reads local system timezones to check if they match restricted zones (such as `Asia/Shanghai` or `Asia/Urumqi`).
   - 本地读取系统的时区设置，检测是否属于受限时区（例如中国标准时区 `Asia/Shanghai` 或 `Asia/Urumqi`）。

2. **Gateway Hostname Verification / 中转站域名审查**
   - Evaluates configured custom API endpoints (`ANTHROPIC_BASE_URL`) against a decrypted internal blacklist to block domestic clouds, proxy relays, and custom forwarding gateways.
   - 当检测到配置了自定义的 API 端点（`ANTHROPIC_BASE_URL`）时，会解密其内部硬编码的 147 个域名黑名单列表进行比对，阻断连接到特定的国内云代理或中转网关。

3. **Prompt Steganography / 提示词隐写水印注入**
   - Replaces standard ASCII apostrophes (`\u0027`) with alternative Unicode equivalents (`\u2019`, `\u02BC`, `\u02B9`) within generated prompts to embed a silent geographic watermark sent to the backend.
   - 在编译发往后端的 Prompt 文本时，将部分标准的 ASCII 撇号（`\u0027`）替换为特定变体的 Unicode 撇号（如 `\u2019`、`\u02BC`、`\u02B9`），向服务器发送隐密的本地区域水印。

---

## Features / 功能特性

- **Zero External Dependencies / 零外部依赖**
  - Compiled into a single, standalone machine binary in Rust. No external runtimes required.
  - 编译为单个独立的 Rust 原生二进制文件，无需 Node.js、Python 等外部运行环境。

- **Surgical Byte Patching / 微创字节修补**
  - Patches timezone strings, telemetry configs, and domain blacklist decryptions in-place using same-length overrides to keep execution signature intact.
  - 原位替换时区字符串、限制特征段和域名解密密钥，采用无损等长替换，确保不破坏原程序的签名与执行完整性。

- **Auto-Scrubbed Execution Wrapper / 自动脱敏运行外壳**
  - Shell aliases dynamically enforce clean locales (`TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8`) upon invocation, completely isolating the process from host environment profiles.
  - 自动挂载终端别名，在启动时为客户端强制套用隔离环境（`TZ=UTC LANG=en_US.UTF-8`），阻断本地系统语言特征泄露。

- **Terminal Proxy Auditing / 终端代理健康审计**
  - Inspects active terminal sessions for proxy variables (`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`) to visually alert developers of potential real IP leaks.
  - 自动审计当前 Shell 终端的代理变量配置，防止因代理未生效导致真实公网 IP 直接泄露给服务端。

- **Lightweight Sentinel Daemon / 实时监控守护进程**
  - Monitors client updates using native OS file events with zero CPU/IO overhead when idle, automatically reapplying hot-patches upon package updates.
  - 通过原生系统文件事件监控目标文件，无更新时 CPU/IO 损耗为零，自动应对 `npm` 包升级并即时重新修补。

---

## How to Use / 如何使用

### 1. Compile & Build / 编译与构建

Make sure you have Rust toolchain installed, then clone and compile:

确保系统已配置 Rust 编译工具链，然后执行拉取与编译：

```bash
git clone https://github.com/CACEB001/Claude-Shield.git
cd Claude-Shield
cargo build --release
```

The compiled binary will be located at `target/release/claude-shield`.

编译出的二进制文件位于 `target/release/claude-shield`。

### 2. Register Commands / 注册全局快捷别名

To run the tool from anywhere in your shell, register the global command alias:

在当前系统中挂载全局别名，以便在任何目录下直接调用：

```bash
./target/release/claude-shield install

# Reload your shell configuration / 重新加载终端配置
source ~/.zshrc  # or ~/.bashrc
```

Now you can invoke the utility from any terminal session via `CLAUDE SHIELD` or `claude-shield`.

挂载完成后，可在任意终端会话中通过 `CLAUDE SHIELD` 或 `claude-shield` 调用。

### 3. Commands Reference / 命令指南

#### Scan / 扫描系统安装
Searches for all global installations and checks the protection status of the Claude Code executables:
扫描系统中所有全局安装的版本并输出当前的防护安全仪表盘：
```bash
claude-shield scan
```

#### Patch / 执行安全修补
Applies same-length byte patches to all detected targets and creates a `.backup` copy:
对扫描出的可修补目标执行一键无损字节修补，并自动备份原始文件：
```bash
claude-shield patch
```

#### Restore / 还原原始备份
Restores the original binary state from backup files:
从备份中恢复所有被修改的目标文件：
```bash
claude-shield restore
```

#### Cloud Protection Configuration / 云端防护配置
Configure and persist midstream API gateways, global network proxy endpoints, and toggle launch enforcement:
配置并持久化管理中转网关 API 端点、全局网络代理，并控制是否开启防 IP 泄露的强制拦截机制：
```bash
# 查看当前的云端防护参数配置
claude-shield config

# 绑定全局网络代理地址
claude-shield config --proxy http://127.0.0.1:7890

# 绑定自定义中转 API 节点（启动客户端时会自动注入重定向）
claude-shield config --base-url https://your-custom-gateway.com

# 开启/关闭断路保护（如果未检测到活跃代理，则强制拦截并中止启动）
claude-shield config --enforce true  # 或者 false

# 清空并重置所有防护配置
claude-shield config --clear
```

#### Sentinel Daemon / 运行实时守护哨兵
Keeps the protection active in the background. It automatically intercepts and hot-patches Claude Code whenever it gets updated or reinstalled:
在后台静默运行守护进程，检测到 `npm` 升级客户端后会自动在毫秒级内自动重新修补：
```bash
# Start the daemon silently / 后台静默启动守护进程
claude-shield start

# Check sentinel daemon running status / 检查守护进程运行状态
claude-shield status

# View background runtime logs / 查看后台运行日志
claude-shield logs

# Stop the daemon / 停止后台守护进程
claude-shield stop
```

---

### Potential Leak Risks & Security Recommendations / 潜在泄露风险与安全建议

Although Claude Shield purges local client environments and translates prompts at the local gateway level, cloud backend security still heavily depends on your network settings:
尽管 Claude Shield 净化了本地客户端并实现了本地中转层的全自动翻译，云端防线依然依赖于您的代理网络配置：

1. **DNS Leak Prevention / 防止 DNS 解析泄露**
   - **Risk**: Your local OS may resolve target API hostnames via domestic ISP DNS resolvers before routing traffic to the proxy, leaking your geographic location history.
   - **风险**: 您的系统在向代理网关发送请求前，可能会通过本地运营商的 DNS 解析目标域名，从而在解析阶段暴露您的真实地理位置。
   - **Solution**: Configure your proxy client (e.g., Clash, v2ray) to use **Remote DNS resolution** or run under global **TUN mode**.
   - **建议**: 请确保您的代理客户端开启了**远程解析 (Remote DNS)**，或者采用全局 **TUN 虚拟网卡模式**。

2. **TLS Fingerprints (JA3/JA4) / TLS 指纹特征安全**
   - **Risk**: The JS proxy agent in Claude Shield uses standard Node.js HTTPS request modules. Cloudflare or API firewalls might inspect TLS Hello fingerprints to detect automated agents.
   - **风险**: 本地中转代理基于 Node.js 发起连接，其 TLS 握手特征与原生浏览器或官方默认配置存在差异，可能被云端防火墙标记为机器人。
   - **Solution**: Route your proxy traffic through a reliable outbound gateway that obfuscates and masquerades TLS client hello handshakes.
   - **建议**: 使用具有 TLS 混淆/模拟功能的优质代理节点进行出海出站连接。

3. **Global Telemetry Bypass / 旁路遥测流量防漏**
   - **Risk**: Diagnostic requests, metric tools, or crash reporters inside Claude Code CLI might request remote hosts bypassing the default API base URL settings.
   - **风险**: 客户端内部的崩溃日志收集或网络状态诊断等旁路遥测流量，可能会绕过 Base URL 重定向设置直连云端。
   - **Solution**: Ensure your proxy gateway runs a global route rule directing all outbound `*.anthropic.com` traffic through the secure tunnel.
   - **建议**: 使用代理网关设置全局路由规则，强制将所有涉及 `*.anthropic.com` 的出站连接全部交由代理通道托管。

---

## License / 开源协议

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
