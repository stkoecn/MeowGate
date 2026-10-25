# MeowGate 🐱 - Clash 节点用户路由网关

> 基于 Rust (Axum + Tokio + SQLite + rustls) 的轻量级、高性能多租户 Clash 节点路由网关与代理调度器。
> 原生兼容 Mihomo (Clash Meta) 内核，全平台跨架构支持（Windows、Debian、Ubuntu、macOS、ARM64）。

---

## 🌟 核心设计理念

传统的 Clash 客户端只是单用户单出口代理，而 **MeowGate** 是为多租户、分布式爬虫、多环境测试和智能分流打造的**超级网关层**：
1. **单端口全协议多路复用（All-in-One Port :9565）**：同一端口同时承载 **Web 管理控制台**、**HTTP 代理**、**SOCKS5 代理** 以及 **Clash/Mihomo 原生 RESTful API**（完美兼容 ZashBoard / Yacd / Metacubexd）。
2. **凭据驱动路由（Credential-Driven Routing）**：
   - 规则：`密码 == 用户名`。
   - 每一个去重后的物理节点分配专属账号（如 `us3e5d24be`），带账号请求即可精准锁定目标物理出口！
3. **免密透明回退（Transparent Guest Fallback）**：
   - 不带任何账号密码的常规连接自动走主选择器 **`🚀 节点选择`**，与标准 Clash 体验 100% 一致。
4. **四维多级智能聚合池**：
   - **地区池**：`hk` (自动选优), `hk-fb` (故障转移), `us`, `jp`, `sg`...
   - **分级池**：`lv1` (极速 <200ms), `lv2` (良好 200~500ms), `lv3` (普通 500~1000ms), `lv4`, `lv5`
   - **速度池**：`fast` (高速 ≥1MB/s), `mid` (中速 300K~1MB/s), `slow` (低速 <300KB/s)
   - **同出口 IP 池**：按探测出的公网出口 IP 自动归拢聚类，支持 `IP_xxx` 独立调度（配置内默认 `hidden: true`，不干扰看板浏览）。

---

## 🏗️ 架构概览

```
客户端请求 (浏览器 / 爬虫 / 脚本 / 第三方客户端 / 手机 / 局域网设备)
               │
               ▼  :9565 (单端口网关绑定 0.0.0.0)
┌─────────────────────────────────────────────────────────────┐
│ MeowGate Gateway (Axum + GatewayMux 协议嗅探层)             │
├───────────────────────────────┬─────────────────────────────┤
│ 1. 网页管理流量 (GET /)       │ 2. 代理流量 (HTTP / SOCKS5) │
│    ➜ 内嵌 Axum Web 控制台     │    ➜ 纳秒级透传至内核混合口 │
│ 3. REST API 监控流量          │    ➜ 免密自动转 guest 注入  │
│    ➜ 原生反代给内核 :9091     │    ➜ 有凭证直接匹配 IN-USER │
└───────────────────────────────┴──────────────┬──────────────┘
                                               ▼
┌─────────────────────────────────────────────────────────────┐
│ 官方 Mihomo (Clash Meta) 完整内核                           │
├─────────────────────────────────────────────────────────────┤
│ 【分层规则流水线】                                          │
│  ① 第一层: IN-USER 规则 (节点/地区/分级/速度账号)            │
│  ② 第二层: 原生分流规则 (Base Config 用户基础模板)          │
│  ③ 第三层: MATCH 兜底 (默认路由到 🚀 节点选择)              │
├─────────────────────────────────────────────────────────────┤
│ 【核心优化特性】                                            │
│  • 全并发 Fake-IP 国内高可用 DNS (阿里/腾讯/Cloudflare)     │
│  • 策略组 lazy: true (空闲不主动测速，CPU 待机 0%)          │
│  • tcp-concurrent 并发连接与 unified-delay 消除偏差         │
└─────────────────────────────────────────────────────────────┘
```

---

## ⚡ 核心功能特性

### 1. 节点采集与去重
* **多配置源管理**：支持从本地目录自动扫描 Clash 配置文件（如 `profiles/*.yaml`）、本地 Clash REST API、以及手动添加/粘贴单文件或目录。
* **物理级去重**：按 `server:port:proxy_type` 全局去重，同物理节点跨配置合并。
* **按需扫描**：扫描仅在手动点击“扫描本机配置”时执行，杜绝后台死循环扫描；已删除源持久化不再复活。

### 2. 节点健康生命周期与测速
* **实时下载带宽测速**：
  * 支持一键下载 Anycast 真实测速块测出真实吞吐速率（如 `⚡ 202 KB/s`），持久化写入 SQLite。
  * 列表及卡片常驻展示 `⚡ 实测网速` 徽标。
* **历史连接流量统计**：自动采样并统计每个节点的活跃连接数、累计上传/下载流量、峰值速率及最后活动时间。
* **分级滚动健康巡检**：300 秒滚动复查，连续失败熔断剔除，连续成功自动恢复；变更 30 秒冷却防抖热重载。
* **近 10 次连通性状态条**：红/黄/绿/灰方块直观记录最近 10 次探针探测结果。

### 3. 一键系统代理接管（Windows & Debian 双平台）
* **Windows**：通过 WinINet API 与注册表即时切换系统全局代理（`127.0.0.1:9565`），免重启即时生效。
* **Debian / Linux**：通过 GNOME `gsettings` 及写入 `/etc/profile.d/meowgate-proxy.sh` 自动接管桌面与终端环境。
* 顶栏带有醒目的 **`[系统代理: 开/关]`** 状态按键，支持一键切换。

### 4. 专属 Clash 订阅链接与扫码导入
* 访问 `http://<ip>:9565/sub` 可直接获取标准 Clash 订阅 YAML。
* 前端提供居中弹窗展示链接，包含 **“一键导入 Clash”**、**“下载 YAML”** 以及 **离线生成的 SVG 二维码**，方便手机客户端（Clash Verge, Shadowrocket, Stash）扫码一键导入。

### 5. 极致体积与性能优化
* **Gzip 传输层流式压缩**：单页 HTML 压缩至 **35 KB**，全量 1400 节点数据接口压缩至 **77 KB**，秒开响应。
* **流式文本预分配生成**：摒弃 DOM 树序列化，百万字节 YAML 生成耗时从 **2.5s 骤降至 10ms**。
* **静态 proxies 内存缓存**：节点文本在内存缓存，健康巡检时零磁盘 I/O 直传热重载。

---

## 🚀 快速接入指南

### 统一端口 `:9565` 出网示例

```bash
# 1. 普通免密接入（走 🚀 节点选择 默认选中的节点）
curl -x http://127.0.0.1:9565 https://api.ipify.org
curl -x socks5://127.0.0.1:9565 https://api.ipify.org

# 2. 指定单物理节点出网（账号格式：国家代码+短哈希，密码相同）
curl -x http://us3e5d24be:us3e5d24be@127.0.0.1:9565 https://api.ipify.org

# 3. 指定极速延时分级组（Level 1 自动测速选优池 / 故障转移池）
curl -x http://lv1:lv1@127.0.0.1:9565 https://api.ipify.org
curl -x http://lv1-fb:lv1-fb@127.0.0.1:9565 https://api.ipify.org

# 4. 指定高速带宽组（Fast 自动选优池 / 故障转移池）
curl -x http://fast:fast@127.0.0.1:9565 https://api.ipify.org
curl -x http://fast-fb:fast-fb@127.0.0.1:9565 https://api.ipify.org

# 5. 指定国家/地区池（香港 / 日本 / 美国自动选优）
curl -x http://hk:hk@127.0.0.1:9565 https://api.ipify.org
curl -x http://jp:jp@127.0.0.1:9565 https://api.ipify.org
curl -x http://us:us@127.0.0.1:9565 https://api.ipify.org
```

---

## 🛠️ 编译与部署

### 1. 本机编译与运行 (Windows / Linux)

```bash
# 调试运行
cargo run

# 发布版本编译（极度精简产物）
cargo build --release
./target/release/meowgate
```

### 2. 交叉编译为 Debian / Linux 静态单二进制 (纯 musl)

使用 `cargo-zigbuild` 进行零环境污染的跨架构编译：

```powershell
# 1. 运行提供的一键编译脚本
.\build-linux.ps1

# 或者手动执行：
cargo zigbuild --target x86_64-unknown-linux-musl --release
```
编译产物位于 `target/x86_64-unknown-linux-musl/release/meowgate`（仅 8.6MB 纯静态二进制，包含前端 UI 和所有图标，无任何 glibc / openssl 依赖）。

### 3. 作为 Linux systemd 服务运行

```bash
# 复制二进制和配置
sudo cp meowgate /usr/local/bin/
sudo mkdir -p /opt/meowgate
sudo cp config.toml /opt/meowgate/

# 配置并启动 systemd
sudo cp meowgate.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now meowgate.service
```

---

## ⚙️ 配置文件说明 (`config.toml`)

```toml
[server]
bind_addr = "0.0.0.0:8080"

[database]
url = "sqlite://meowgate.db?mode=rwc"

[clash]
api_url = "http://127.0.0.1:9090"
auto_detect_profiles = true
profiles_dir = "/opt/meowgate/profiles"

[meow]
api_url = "http://127.0.0.1:9091"
config_output_path = "meow-config.yaml"
# 唯一对外公开端口 (Web UI + 代理流量 + Clash REST API)
gateway_port = 9565
gateway_bind = "0.0.0.0"
mixed_port = 9567
default_target = "DIRECT"

[health]
enabled = true
interval_secs = 300
timeout_ms = 2500
fail_threshold = 2
recover_threshold = 1
down_retry_secs = 600
concurrency = 32
test_url = "http://cp.cloudflare.com/generate_204"
```

---

## 📄 开源许可证

本项目基于 [MIT License](LICENSE) 开源。
