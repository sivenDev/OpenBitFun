---
id: "F002-10"
title: "局域网共享助理的基础：设备共享身份、Bonjour 发现、加密通道，安装包内带执行器"
type: "feature"
status: "complete"
summary: "提供与账号无关的设备共享身份、局域网内的 Bonjour 广播与发现、端到端加密的设备直连通道与能力协商，并让桌面安装包自带外部任务执行器（修复 B004）。"
source: "docs/superplan/human/features.md"
created: "2026-11-02"
order: 10
depends_on: []
parent: "F002"
---

# 局域网共享助理的基础 Plan

**需求基准：** RFC `docs/superplan/rfcs/F002.md` 第 6 版的「AI协作需求（基准，第 6 版）」，D1（局域网发现）、D2（设备共享身份）、D3（局域网通道）、D9 最后一条（安装包内带执行器），以及「风险与缓解」「兼容性、安全与回滚」。

**Goal:** 为后续的共享、申请与使用打好底座：两台未登录的设备在同一局域网里能互相发现，建立经过身份校验、端到端加密的直接通道；只装了桌面安装包的设备能执行外部任务。

**Scope:**
- **设备共享身份**：
  - 每台设备生成并持久保存一对 X25519 共享身份密钥，与账号、中继无关，不登录也存在。
  - 显示名默认取本机用户全名，可以修改；设备名取主机名。
  - 指纹短码：由公钥派生的 6 位短码，便于人工核对；只用于显示，不用于校验。
  - 设备 id 由公钥派生，任何设备都不能用自己的密钥冒用别人的设备 id。
- **Bonjour 广播与发现**：
  - 用纯 Rust 的 `mdns-sd` 广播与浏览 `_openbitfun._tcp`。
  - TXT 只含：协议版本、设备 id、设备名、显示名、完整共享公钥（指纹短码由它派生）。
  - 广播与浏览都由调用方显式开启和停止：不共享时不广播；只在需要时浏览。
  - 浏览结果是对端的增减事件。
- **局域网通道**：
  - WebSocket over TCP，监听端口只绑定私有网段（RFC 1918）的网卡地址，拒绝来自非私有地址的连接。
  - 握手交换协议版本、能力列表、设备 id、共享公钥与临时公钥。会话密钥由静态与临时 X25519 共同派生（HKDF-SHA256，独立的域标签），之后每帧 AES-256-GCM 加密；每个方向按计数器生成 nonce，拒绝重放和乱序。
  - 通道提供请求 / 响应和服务端推送两种消息。入站请求交给调用方注册的处理器，处理器拿到对端的设备 id 与已校验的公钥。
  - 能力协商：双方声明能力；缺少所需能力时，握手以「需要更新」结束。版本号相同不能代替能力声明。
  - 资源与安全上限：
    - 握手超时；
    - 单帧大小上限；
    - 未完成握手的连接数上限；
    - 对端公钥与 TXT 广播的公钥不一致时拒绝；设备 id 与公钥不对应时拒绝。
- **安装包内带执行器（B004）**：
  - 桌面打包时为同一目标构建 `openbitfun` CLI，作为 sidecar 放进安装包（macOS 在 `Contents/MacOS/`）。
  - 桌面端查找执行器时优先使用安装包内的版本，其次才是 `~/.local/bin` 与 `PATH`。
  - macOS `Info.plist` 声明本地网络用途说明与 `_openbitfun._tcp` Bonjour 服务类型。

**Non-Goals:**
- 共享设置、发现面板、申请与确认框、授权存储、撤销（F002-11）。
- 删除第 5 版的 AI协作页、连接与中继跨账号能力（F002-11）。
- 新增 Tauri 命令或界面；桌面端在本计划中不启动广播与监听（F002-11 随共享入口启动）。
- 使用共享的助理、对话（F002-12）；助理之间相互调用（F002-13）。
- CLI 主机提供共享或发现（RFC D11：不提供）。

**Architecture:**
- **归属**：
  - 身份、发现、通道放在 `openbitfun-services-integrations` 新的 `lan-share` feature 下（services 层：可复用的网络与 OS 能力，不依赖 core 与 app）。
  - 产品语义（谁能申请、授权、确认框）在 F002-11 由 core 与桌面端在其上组装。
  - `lan-share` 不进入默认 feature；是否加入 `product-full` 由 F002-11 接入桌面时决定。
- **依赖**：
  - 新增 `mdns-sd`（0.21.x，Apache-2.0 OR MIT，MSRV 1.71），在根 `[workspace.dependencies]` 声明版本并关闭默认 feature；
  - 复用工作区已有的 `tokio-tungstenite`、`aes-gcm`、`x25519-dalek`、`hkdf`、`sha2`、`whoami`；
  - 成员只开启用到的 feature，不引入 `tokio/full`。
- **密钥材料**：
  - 共享身份存于 `~/.openbitfun/lan-share/identity.json`，权限 0600，原子写入；
  - 读不了时明确报错，不覆盖、不重建；
  - 日志只记录指纹短码，不记录密钥。
- **执行器**：
  - `scripts/desktop-tauri-build.mjs` 按 miniapp shell 的 sidecar 方式构建 CLI 并注入 `bundle.externalBin`；CI 打包经同一脚本，无需改工作流的构建步骤。
  - `dispatch_host.rs` 的候选顺序改为安装包内优先。
- **远程场景**：本计划只交付底座，没有产品入口，远程控制、Peer Device Mode、远程工作区与 Dispatch 不受影响；各场景的入口与不支持状态在 F002-11 起落实。

**Baseline:**
- 仓库没有 mDNS 依赖；现有「局域网模式」（`remote_connect/lan.rs`）只为手机配对构造本机中继地址，不发现其他设备。
- 设备消息密钥（`remote_connect/device_crypto.rs`）是 X25519 + HKDF，与账号会话一起保存，未登录时不可用。
- `dispatch_host.rs` 依次在 `~/.local/bin`、可执行文件所在目录、`PATH` 中查找 `openbitfun`；dmg 中只有 `openbitfun-desktop` 和 `openbitfun-miniapp-shell`，只装 dmg 的设备无法执行外部任务（B004，2026-11-02 真机复现）。

**Exit Criteria:**
- 两个共享身份在同一台机器上经通道完成握手并互发加密请求与推送；篡改、重放、错误公钥、缺少能力、超时、超大帧、非私有地址都被拒绝，并给出明确原因（自动化测试）。
- 两台 Mac（arm64 与 x64）在同一局域网、都未登录账号：一方开启广播，另一方开启浏览后在 10 秒内看到对端的设备名、显示名与指纹短码；停止广播后对端收到移除事件；双方建立通道并完成一次加密往返（`lan_share_probe` 示例程序）。
- 只安装 dmg、未另装 CLI 的 Mac 上，`Contents/MacOS/openbitfun dispatch probe` 返回包含 `agent_delegation_v1` 的能力声明，桌面端选用这个执行器。
- 安装包体积变化记录在证据中。

## Task 1: 设备共享身份

**Outcome:** 每台设备有一对与账号无关的持久共享身份密钥、可读的显示名、设备名与指纹短码；读写安全，读不了时明确报错。

**Files:**
- Modify: `Cargo.toml`（`[workspace.dependencies]` 增加 `mdns-sd`，关闭默认 feature）
- Modify: `src/crates/services/services-integrations/Cargo.toml`（`lan-share` feature 及其依赖）
- Modify: `src/crates/services/services-integrations/src/lib.rs`
- Create: `src/crates/services/services-integrations/src/lan_share.rs`、`src/crates/services/services-integrations/src/lan_share/identity.rs`、`src/crates/services/services-integrations/src/lan_share/tests.rs`
- Modify（边界检查要求时）: `scripts/core-boundaries/` 中的 feature 清单、`src/crates/services/services-integrations/AGENTS.md`（记录 `lan-share` 的测试命令）

**Change Map:**
- `identity.rs`：生成、加载、保存身份；显示名默认取本机用户全名，可修改；指纹短码由公钥的 SHA-256 派生，6 位，去掉易混字符。
- 文件权限 0600，原子写入；损坏时返回错误，不覆盖。

**Verification:**
- `cargo test --locked -p openbitfun-services-integrations --no-default-features --features lan-share --lib lan_share::`
- `pnpm run check:core-boundaries`

- [x] 设备共享身份可生成、持久、读回一致；损坏文件明确报错且不被覆盖；文件权限为 0600
  - 证据（2026-11-02）：
    - `lan_share::identity`：`ShareIdentity::load_or_create` 不存在时生成并经 `remote_persistence::write_private_bytes` 原子写入；读不了时报错「unreadable; move it away」，文件保持原样。
    - 显示名默认取账号全名（Unix 读 passwd 的 GECOS 字段，工作区没有 `whoami`，改用已有的可选依赖 `libc`），取不到时用登录名，再退回设备名；指纹 6 位，字母表去掉 0/1/I/O。
    - 安全复审（自动安全检查）指出：6 位短码约 30 位熵，若用它钉住对端公钥，局域网内可暴力生成同码密钥冒充。已改为：TXT 广播完整公钥并在连接时逐字节比对；设备 id 由公钥的 SHA-256 派生，广播与握手都校验二者对应；短码只用于显示。RFC D1 措辞同步澄清。
    - `lan_share::` 单元测试 16 项通过：持久与 0600、改名保存、损坏文件不覆盖、指纹格式、私有地址判定、广播或握手中设备 id 与公钥不对应即拒绝等。
    - 边界：`lan-share` 登记进 `scripts/core-boundaries` 的 Tokio 能力、tokio-tungstenite（无 TLS）、可选依赖归属与测试目标清单；`pnpm run check:core-boundaries` 通过，`node --test scripts/check-core-boundaries.test.mjs` 156 项通过（夹具加入 `lan-share` 作为已审查的 WebSocket 归属）。

## Task 2: Bonjour 广播与发现

**Outcome:** 调用方可以开启或停止广播本机共享身份，开启或停止浏览，并收到局域网内对端的增减事件；TXT 内容最少，解析严格。

**Files:**
- Create: `src/crates/services/services-integrations/src/lan_share/discovery.rs`
- Create: `src/crates/services/services-integrations/examples/lan_share_probe.rs`
- Modify: `src/crates/services/services-integrations/src/lan_share.rs`、`src/crates/services/services-integrations/src/lan_share/tests.rs`、`src/crates/services/services-integrations/Cargo.toml`（`[[example]]` 需要 `lan-share`）

**Change Map:**
- `discovery.rs`：
  - 服务类型 `_openbitfun._tcp.local.`；TXT 键 `v`、`id`、`name`、`owner`、`pk`。
  - 浏览时丢弃缺键、超长、版本号无法解析、公钥格式错误或设备 id 与公钥不对应的记录；忽略本机自己的广播。
  - 对端以设备 id 去重，携带解析出的地址与端口。
- 示例程序：在一台机器上同时广播与浏览，打印发现的对端，并对第一个对端发起一次通道握手与加密往返（Task 3 完成后接上），用于两台 Mac 的手工验证。

**Verification:**
- `cargo test --locked -p openbitfun-services-integrations --no-default-features --features lan-share --lib lan_share::discovery`
- 手工：两台 Mac 运行 `lan_share_probe`（x64 版交叉编译后拷贝），互相在 10 秒内出现；停止一方后另一方收到移除事件

- [x] TXT 编解码与过滤规则有单元测试；两台 Mac 互相发现并收到移除事件
  - 证据（2026-11-02）：
    - 单元测试覆盖合法记录、更高协议版本（保留并标为不兼容）、缺键 / 设备 id 与实例名或公钥不符 / 公钥格式错 / 版本非数字 / 名称超长或为空 / 无私有地址 / 端口 0 均丢弃。
    - 真机首轮发现 Bonjour 刷新记录时同一设备会重复报告「发现」，新增 `PeerSet` 按设备 id 去重（仅新设备或信息变化时报告），有单元测试。
    - 两台 Mac（本机 arm64 macOS 27，172.16.21.153；shimo x64 macOS 12.7.6，172.16.22.255），均未登录、互不依赖中继，`lan_share_probe` 同时运行：本机 0.1 秒、对端 2.9 秒看到对方的设备名、显示名与指纹；本机停止广播约 1 秒后对端收到移除事件；第二次运行沿用已保存的身份（id 与指纹不变）。改为完整公钥钉住后复测：本机 0.0 秒、对端 2.9 秒互相发现，停止后约 1 秒收到移除事件。

## Task 3: 加密的局域网通道与能力协商

**Outcome:** 两个共享身份之间可以建立经过校验、端到端加密的直接通道，支持请求 / 响应与推送；不合规的连接一律被拒绝，并给出原因。

**Files:**
- Create: `src/crates/services/services-integrations/src/lan_share/channel.rs`、`src/crates/services/services-integrations/src/lan_share/handshake.rs`
- Create: `src/crates/services/services-integrations/tests/lan_share_contracts.rs`
- Modify: `src/crates/services/services-integrations/src/lan_share.rs`、`src/crates/services/services-integrations/examples/lan_share_probe.rs`

**Change Map:**
- `handshake.rs`：
  - hello 帧明文交换协议版本、能力、设备 id、共享公钥、临时公钥；
  - 会话密钥 = HKDF-SHA256(静态 DH ‖ 临时 DH，salt = 双方临时公钥，info = 域标签与双方设备 id)，每个方向一把；
  - 校验对端公钥与发现时广播的公钥完全一致（发起方提供期望公钥时），且设备 id 由该公钥派生。
- `channel.rs`：
  - 监听只绑定私有网段网卡地址，测试可以注入回环地址；拒绝非私有地址的入站连接。
  - 帧：AES-256-GCM，nonce 由方向与递增计数器组成，计数器不连续即断开。
  - 消息：带 id 的请求 / 响应，以及服务端推送。入站请求交给注册的处理器，处理器拿到对端设备 id 与公钥。
  - 上限：握手超时、单帧大小、未完成握手的连接数。
  - 能力协商：调用方声明所需能力，缺失时以「需要更新」结束。

**Verification:**
- `cargo test --locked -p openbitfun-services-integrations --no-default-features --features lan-share --test lan_share_contracts`
- `cargo test --locked -p openbitfun-services-integrations --no-default-features --features lan-share --lib lan_share::`
- 手工：两台 Mac 用 `lan_share_probe` 完成握手与一次加密往返
- `pnpm run lint:rs`

- [x] 握手、加密往返与推送通过；篡改、重放、错误公钥、缺少能力、超时、超大帧、非私有地址被拒绝，且原因明确；两台 Mac 真机往返成功
  - 证据（2026-11-02）：
    - `lan_share_contracts` 7 项通过（连续 5 轮稳定）：双向请求、推送、关闭后调用返回 `Closed`；对端公钥与期望公钥不同 `PeerKeyMismatch`；缺少能力两侧分别得到 `NeedsUpdate` / `PeerNeedsUpdate`；对端不说话 `Timeout`；未完成握手的连接占满名额时新连接被拒、超时后恢复；超大请求 `TooLarge`、超大响应以明确错误返回且连接仍可用；非私有地址的连接与监听被拒绝。
    - 单元测试覆盖帧篡改、重放与乱序、跨会话帧、与自身身份握手。
    - 两台 Mac 真机：按广播的完整公钥钉住后完成加密往返，本机→对端 2–3 ms，对端→本机 66–129 ms。
    - `cargo clippy -p openbitfun-services-integrations --no-default-features --features lan-share --lib --tests --examples` 新代码无告警（唯一告警是 `openbitfun-core-types` 中已有的 `derivable_impls`）；未跑全工作区 `pnpm run lint:rs`。

## Task 4: 安装包内带执行器

**Outcome:** 桌面安装包自带 `openbitfun` CLI；桌面端优先使用它，只装 dmg 的设备能执行外部任务。macOS 安装包声明本地网络用途。

**Files:**
- Modify: `scripts/desktop-tauri-build.mjs`、`scripts/desktop-tauri-build.test.mjs`
- Modify: `src/apps/desktop/src/api/dispatch_host.rs`
- Modify: `src/apps/desktop/Info.plist`
- Modify: `src/apps/desktop/README.md`（安装包内容与执行器查找顺序）

**Change Map:**
- 构建脚本：为目标三元组构建 `openbitfun-cli` 的 release 二进制，按 Tauri sidecar 命名放到 sidecar 目录，注入 `bundle.externalBin`；`--bundle-only` 时要求二进制已存在，缺失时明确报错。
- `dispatch_host.rs`：候选顺序改为可执行文件所在目录、`~/.local/bin`、`PATH`；增加顺序的单元测试。
- `Info.plist`：`NSLocalNetworkUsageDescription`（英文，说明用于在局域网内发现与共享助理）、`NSBonjourServices` = `_openbitfun._tcp`。

**Verification:**
- `node --test scripts/desktop-tauri-build.test.mjs`
- `cargo test --locked -p openbitfun-desktop --lib api::dispatch_host`
- 手工：按 1.0.8-plus 的方式打 arm64 与 x64 dmg；在没有 `~/.local/bin/openbitfun` 的两台 Mac 上安装，`Contents/MacOS/openbitfun dispatch probe` 输出能力声明；记录 dmg 体积变化

- [x] 安装包内带执行器且被优先使用；只装 dmg 的 Mac 能回答 `probe`；`Info.plist` 声明本地网络用途；体积变化已记录
  - 证据（2026-11-02）：
    - `node --test scripts/desktop-tauri-build.test.mjs` 27 项通过；`cargo test --locked -p openbitfun-desktop --lib api::dispatch_host` 4 项通过，其中 `the_bundled_executor_is_preferred_over_installed_ones` 覆盖查找顺序（可执行文件所在目录、`~/.local/bin`、`PATH`）。桌面端在运行中实际选用安装包内执行器，留到 F002-11/12 的真机验收中经外部任务再次确认。
    - 按 1.0.8-plus 的方式打包：arm64 与 x64 的 `Contents/MacOS/` 都有 `openbitfun`（arm64 113,951,952 字节，x64 129,725,456 字节），`codesign --verify --deep --strict` 通过；两个包的 `Info.plist` 都含 `NSLocalNetworkUsageDescription` 与 `NSBonjourServices` = `_openbitfun._tcp`。
    - 本机（arm64，未装 CLI）从挂载的 dmg 运行 `Contents/MacOS/openbitfun dispatch probe`：能力中有 `agent_delegation_v1`。
    - 另一台 Mac（x64，macOS 12.7.6，无 `~/.local/bin/openbitfun`、`PATH` 中无 `openbitfun`）安装 x64 dmg 后运行同一命令：`x86_64`、`cliVersion` 1.0.8、含 `agent_delegation_v1`；应用启动后保持运行。
    - dmg 体积：arm64 122,587,788 → 172,480,684 字节（+49.9 MB，+40.7%）；x64 132,169,409 → 188,165,531 字节（+56.0 MB，+42.4%）。

## 风险

| 风险 | 缓解 |
|---|---|
| 安装包体积增加（CLI release 约 115 MB，dmg 预计增加约 40–50 MB） | 记录实际增量；如不可接受，后续单独评估让桌面可执行文件直接承担执行器（需要把 Dispatch 执行器从 CLI 下沉为共享 crate，范围较大，不在本计划内） |
| mDNS 在部分网络（访客 Wi-Fi、隔离 AP、VPN）中被屏蔽 | 本计划只交付能力；发现为空时的原因说明在 F002-11 |
| 监听端口暴露在局域网 | 只在共享时监听（F002-11 启动）；只绑定私有网段；握手前不处理任何业务请求；未完成握手的连接数与帧大小有上限 |
| 加密实现错误 | 使用工作区已审查的原语；静态与临时密钥共同派生；按方向计数器防重放；合约测试覆盖篡改与重放 |

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/human/bugs.md`（B004）
- `src/crates/services/services-integrations/AGENTS.md`
- `src/apps/desktop/AGENTS.md`
