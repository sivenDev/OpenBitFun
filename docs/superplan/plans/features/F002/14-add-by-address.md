---
id: "F002-14"
title: "按地址添加：不同网段的设备按局域网地址添加、核对设备码、记住并自动重连"
type: "feature"
status: "in_progress"
summary: "Bonjour 不跨网段。主人看到本机的局域网地址与设备码；使用者在「发现」中输入地址，核对设备码后添加该设备，之后申请、确认、对话、在线状态与推送与 Bonjour 发现的设备一致；添加过的设备记住地址与公钥，在后台自动重连。"
source: "docs/superplan/human/features.md"
created: "2026-11-03"
order: 14
depends_on: ["F002-11"]
parent: "F002"
---

# 按地址添加 Plan

**需求基准：** RFC `docs/superplan/rfcs/F002.md` 第 7 版：基准第 3 条；目标 10；非目标（只接受私有地址、不自动扫描其他网段）；D1（Bonjour 只在一个网段内）、D3（固定默认端口）、D13；「交互设计要点 · 按地址添加」；风险表中按地址添加的四项；批准条件 11。

**Goal:** 两台 Mac 在同一局域网的不同网段（Bonjour 互相看不到）：A 共享助理 X 后看到本机地址与设备码；B 在「发现」中按地址添加 A，核对设备码后看到 X 并申请，A 允许后 X 进入 B 的列表；撤销、不在线与恢复都与 Bonjour 发现的设备一样及时；每种出错都有一句原因。

**Scope:**
- **固定默认端口**：通道优先监听默认端口 `47652`（执行时核对未被常见服务占用；若冲突，在本计划中换一个并记录）；默认端口在任一私有地址上被占用时，整体改用随机端口（沿用现有逻辑）。Bonjour 广播照常带端口。
- **本机地址与设备码（主人与使用者都可见）**：
  - `HostStatus` 增加本机的私有 IPv4 地址列表（每个网卡一个）；监听在默认端口时显示为 `地址`，否则显示为 `地址:端口`；没有监听时不显示地址。
  - 共享管理视图与「发现」面板底部显示这些地址与本机设备码，可复制，并说明「其他网段的设备可以在『发现』中输入这个地址」。
- **按地址添加（使用者）**：
  - 「发现」面板底部的「按地址添加」：点开后输入地址（可带端口），点「连接」。
  - 地址校验：只接受 IPv4 私有地址（10/8、172.16/12、192.168/16）与链路本地地址（169.254/16）；域名、公网地址、回环地址、格式错误直接拒绝并说明。
  - 连接不带预期公钥，握手完成后取得对方的设备名、显示名、设备码；同时读取目录（有几个共享的助理）。
  - 显示核对步骤：「对方设备：某某（设备名）· 设备码 XXXXXX。请核对对方屏幕上显示的设备码」，按钮「添加」「取消」。确认后保存并加入「发现」列表，标「按地址添加」。
  - 之后的申请、确认框、倒计时、冷却与结果推送与 Bonjour 发现的设备完全相同。
- **记住的设备**：
  - 保存在使用者本机 `<product home>/lan-share/added-devices.json`：公钥、设备 id、显示名、设备名、设备码、地址、添加时间。只在本机，不同步。
  - 之后连接一律带预期公钥；公钥不一致时拒绝，该设备显示「对方设备的身份已变化」，需要移除后重新添加。
  - 「发现」打开时，记住的设备与 Bonjour 发现的设备一起列出并读取目录；同一公钥的设备被 Bonjour 发现时，以 Bonjour 的地址为准并更新记住的地址，列表中只出现一次。
  - 「发现」面板中可以移除记住的设备，或修改其地址（重新连接并核对公钥一致）。移除记住的设备不影响已经授权的助理；助理照常可用，只是没有 Bonjour 时会显示不在线，直到使用者重新添加。
- **后台重连**：有可用的来自共享的助理时，后台每 30 秒检查一次这些助理的主人；通道未打开的，用 Bonjour 地址、记住的地址、上次连接的地址依次重连，连上后读取目录（补上离线期间的撤销）并更新在线状态。Bonjour 发现的设备同样受益（错过 mDNS 公告时也能恢复）。
- **出错说明（一句原因）**：
  - 地址格式不对；不是局域网地址（公网、域名、回环）；
  - 连接不上：「无法连接到 X。请确认对方已打开 OpenBitFun 并共享了助理；如果开着代理、VPN 或防火墙，可能被它拦截」；
  - 对方不是 OpenBitFun（握手失败）；需要更新（版本不兼容）；
  - 对方没有共享任何助理（目录为空，仍允许添加）；对方设备的身份已变化。

**Non-Goals:**
- IPv6 地址：通道目前只监听 IPv4（`sharing_ipv4_addresses`），本计划只接受 IPv4；输入 IPv6 时说明「暂只支持 IPv4 地址」。与 RFC D13 列出的 IPv6 ULA 不一致，作为差异记录，IPv6 监听以后单独做。
- 自动扫描其他网段；跨互联网或经中继（RFC 非目标）。
- 手机端、IM、CLI 主机上的入口（RFC D13 远程场景）；Peer Device Mode 下只显示「对端模式下不可用」，不提供按地址添加。
- 主人一侧主动添加使用者设备（只有使用者添加主人）。

**Architecture:**
- **归属**：
  - `services-integrations::lan_share`：`bind_all` 先尝试默认端口再退回随机端口；导出默认端口常量；地址解析与私有地址校验（复用 `is_sharing_address`）。只提供机制，不含产品语义。
  - `core::service::lan_share`：
    - `added_devices.rs`（新）：记住的设备存储，读不了时报告且不覆盖。
    - `browse.rs`：发现列表的设备增加来源（`bonjour` / `address`）；记住的设备作为同一种设备条目进入列表，按公钥去重。
    - `host.rs`：`probe_address`（不带预期公钥连接、取对方身份与目录，不保存）、`add_device`（带对方公钥重连确认一致后保存并加入列表）、`remove_device`、`set_device_address`；后台重连任务；`HostStatus.addresses`。地址来源顺序：Bonjour、记住的地址、`shared_with_me.last_address`。
  - 桌面端 `lan_share_api.rs`：新命令 `lan_share_probe_address`、`lan_share_add_device`、`lan_share_remove_device`、`lan_share_set_device_address`（结构化 `request`），在 Product Operation Registry 中登记为 `Agnostic`、`ControllerLocal`、`REFUSED`，与其他 `lan_share_*` 命令一致；重新生成能力目录。
  - Web UI：`LanShareAPI` 增加对应方法与类型；`DiscoverPanel` 底部的本机地址与「按地址添加」；`AssistantShareManageDialog` 显示本机地址与设备码；设备行标「按地址添加」并提供移除与修改地址。
- **安全**：按地址添加的首次连接不带预期公钥，只用于取得对方身份；保存前必须由使用者核对设备码；保存后固定公钥。只连接用户输入的一个地址与端口，不扫描。主人侧不变：未授权的对端只能读目录与申请，每个申请仍须主人当场确认。
- **远程场景**：新命令为 `ControllerLocal`，不代理到对端；Peer Device Mode 下「按地址添加」与本机地址显示「对端模式下不可用」；手机端、IM、CLI 无入口；Detached Dispatch 与通道协议版本不变（只是连接方式不同，不新增能力）。
- **不做兼容**：F002 上线前不考虑兼容；`added-devices.json` 是新文件。

**Baseline:**
- 通道：`services-integrations/src/lan_share/channel.rs` 的 `bind_all`（随机端口）、`connect(config, address, expected_public_key, handler)`（`expected_public_key` 为 `None` 时接受任意对端并返回 `PeerIdentity`）。
- 主机：`core/src/service/lan_share/host.rs` 的 `owner_connection`、`owner_addresses`（已用 `last_address`）、`sync_background`/`kept_owners`（F002-12 Task 4）、`request()` 连不上时把设备标为不在线（626cc7a3f）。
- 界面：`DiscoverPanel.tsx`（底部已有「你在其他设备上显示为…」）、`AssistantShareManageDialog.tsx`。
- 实测：2026-11-03 本机 172.16.23.235/22 与 agent002 172.17.9.12/24 之间 ping、SSH、屏幕共享可通，Bonjour 双向不可见；本机到 172.17.9.0/24 的路由经过代理的 TUN 网卡（198.18.0.1）。

**Exit Criteria:**
- RFC 批准条件 11：本机（172.16.23.235）与 agent002（172.17.9.12）之间，A 共享助理 X 后在共享管理视图看到本机地址与设备码；B 在「发现」中按地址添加 A，核对设备码后看到 X 并申请，A 允许后 X 进入 B 的列表；A 撤销后 B 立即显示「已撤销」；A 退出后 B 显示「不在线」，A 重新打开后自动恢复；输入公网地址、错误格式、不可达地址或身份变化的设备时，B 都看到一句话原因。
- 默认端口与退回、地址校验、按地址探测与添加、公钥固定与身份变化、记住的设备与 Bonjour 去重、后台重连、移除与修改地址均有自动化测试；局域网部分有两主机回环测试。
- Peer Device Mode 下入口不可用有组件测试。

## Task 1: 固定默认端口与本机地址

**Outcome:** 通道优先监听默认端口，占用时退回随机端口；主机报告本机的私有地址与设备码，界面可以展示。

**Files:**
- Modify: `src/crates/services/services-integrations/src/lan_share/{channel.rs,discovery.rs,tests.rs}`、`src/crates/services/services-integrations/src/lan_share.rs`
- Modify: `src/crates/assembly/core/src/service/lan_share/host.rs`

**Change Map:**
- `bind_all(addresses, preferred_port)`：先在所有地址上绑定默认端口，任一失败则全部释放后走原有的随机端口逻辑；`ChannelConfig` 带默认端口，测试可设为 0。
- `HostStatus.addresses`：监听时列出本机私有 IPv4 地址，默认端口时不带端口。

**Verification:**
- `cargo test --locked -p openbitfun-services-integrations --features lan-share lan_share`
- `cargo test --locked -p openbitfun-core --features lan-share --lib lan_share`

- [ ] 默认端口可用时使用默认端口，占用时退回随机端口；本机地址按规则显示

## Task 2: 按地址探测、添加与记住的设备

**Outcome:** 使用者可以按地址探测对方、核对后添加；记住的设备固定公钥，与 Bonjour 发现的设备合并在同一列表中，可以移除与修改地址。

**Files:**
- Create: `src/crates/assembly/core/src/service/lan_share/added_devices.rs`
- Modify: `src/crates/assembly/core/src/service/lan_share/{host.rs,browse.rs,mod.rs,tests.rs}`、`src/crates/assembly/core/tests/lan_share_flow.rs`

**Change Map:**
- 地址解析：`地址` 或 `地址:端口`，无端口时用默认端口；只接受 IPv4 私有与链路本地地址，其余返回明确的原因。
- `probe_address`：不带预期公钥连接，返回对方身份、设备码、共享助理数与协议是否兼容；不保存，探测用的通道随即关闭。
- `add_device(address, public_key)`：带公钥连接确认一致后保存，加入发现列表（来源 `address`），并读取目录。
- 发现列表按公钥去重；Bonjour 发现同一公钥时更新记住的地址。
- 连接时公钥不一致：设备状态为「身份已变化」，不读取目录。
- `remove_device`、`set_device_address`（新地址必须是同一公钥）。

**Verification:**
- `cargo test --locked -p openbitfun-core --features lan-share --lib lan_share`
- `cargo test --locked -p openbitfun-core --features lan-share,dispatch-store,agent-runtime,ssh-remote,git --test lan_share_flow`

- [ ] 回环测试：关闭 Bonjour 时，使用者按地址探测、添加主人，申请并获准；公钥变化时拒绝；记住的设备在重新打开「发现」时列出；与 Bonjour 发现的同一设备只出现一次

## Task 3: 后台重连

**Outcome:** 有可用的来自共享的助理时，未连上的主人每 30 秒按已知地址重连；连上后在线状态恢复，离线期间的撤销被补上。

**Files:**
- Modify: `src/crates/assembly/core/src/service/lan_share/host.rs`、`src/crates/assembly/core/tests/lan_share_flow.rs`

**Change Map:**
- `sync_background` 在需要保持时启动重连任务，不需要时停止；重连间隔可由选项设置，测试中缩短。
- 重连成功后投递待发送的移除并读取目录（`sync_grants` 标出撤销）。

**Verification:**
- `cargo test --locked -p openbitfun-core --features lan-share,dispatch-store,agent-runtime,ssh-remote,git --test lan_share_flow`

- [ ] 回环测试：主人退出后使用者显示不在线；主人在同一端口重新启动后，不经 Bonjour 自动恢复在线；主人离线期间的撤销在重连后标出

## Task 4: 桌面端命令与注册表

**Outcome:** Web UI 经 Tauri 命令探测、添加、移除与修改记住的设备；命令在注册表中登记。

**Files:**
- Modify: `src/apps/desktop/src/api/lan_share_api.rs`、`src/apps/desktop/src/api/mod.rs`、`src/apps/desktop/src/lib.rs`
- Modify: `src/crates/contracts/product-domains/src/remote_surface/table.rs` 与生成的注册表、`src/shared/interactive-capabilities/catalog.json` 与生成物、`docs/interactive-capabilities/`

**Change Map:**
- `lan_share_probe_address`、`lan_share_add_device`、`lan_share_remove_device`、`lan_share_set_device_address`：结构化 `request`；注册表行 `Agnostic`、`ControllerLocal`、`REFUSED`。

**Verification:**
- `cargo test --locked -p openbitfun-desktop --lib api::lan_share_api`
- `cargo test --locked -p openbitfun-product-domains --lib remote_surface`
- `node scripts/generate-interactive-capabilities.mjs --check`、`node --test scripts/interactive-capabilities.test.mjs`
- `pnpm run check:core-boundaries`

- [ ] 命令与注册表一致，生成物检查通过

## Task 5: 界面：本机地址、按地址添加与记住的设备

**Outcome:** 主人在共享管理视图与「发现」面板看到本机地址与设备码；使用者在「发现」面板按地址添加、核对设备码、移除或修改记住的设备；出错时有一句原因。

**Files:**
- Modify: `src/web-ui/src/infrastructure/api/service-api/LanShareAPI.ts`、`src/web-ui/src/infrastructure/lan-share/lanShareStore.ts`
- Modify: `src/web-ui/src/app/scenes/profile/views/{DiscoverPanel.tsx,AssistantShareManageDialog.tsx}` 与对应 `.scss`、外观文件和测试
- Modify: 三种语言的 `scenes/profile.json`

**Change Map:**
- 「发现」底部：本机地址与设备码（可复制）；「按地址添加」默认收起，展开为输入框与「连接」；探测结果以核对步骤显示，「添加」后关闭。
- 设备行：来源为地址时标「按地址添加」，提供「修改地址」「移除」；身份变化时显示原因。
- 共享管理视图：本机地址与设备码。
- Peer Device Mode 下入口禁用并说明。
- 文案不出现「连接」「授权」「中继」等内部词以外的新术语；出错文案逐条对应 Scope。

**Verification:**
- `pnpm --dir src/web-ui run test:run src/app/scenes/profile src/infrastructure/lan-share`
- `pnpm run type-check:web`、`pnpm run lint:web`、`pnpm run i18n:audit`、`pnpm run theme:color-audit:all`

- [ ] 按地址添加、核对、出错、记住的设备操作、本机地址显示与 Peer Device Mode 禁用有组件测试

## Task 6: 不同网段两台 Mac 真机验收

**Outcome:** 按 Exit Criteria 在本机（arm64，172.16.23.235/22）与 agent002（arm64，172.17.9.12/24）上验收按地址添加的全过程。

**Files:**
- Modify: `docs/superplan/plans/features/F002/14-add-by-address.md`（证据）

**Change Map:**
- 打 arm64 dmg 分别安装（本机用隔离数据目录的副本）；用 cua-driver 驱动本机界面，用屏幕共享驱动 agent002；两个方向各添加一次（各自做主人与使用者）。
- 记录代理 TUN 路由对连接的影响：若本机经代理连不上，记录现象与提示是否准确。

**Verification:**
- 手工：Exit Criteria 第 1 条逐项记录。

- [ ] 不同网段两台 Mac 验收逐项通过，或记录失败与修复

## 风险

| 风险 | 缓解 |
|---|---|
| 默认端口与其他软件冲突 | 冲突时退回随机端口，本机地址显示端口；执行时核对端口 |
| 代理 TUN 接管到其他网段的路由 | 出错文案提示代理、VPN、防火墙；验收中两个方向都测 |
| 记住的设备地址随 DHCP 变化 | Bonjour 可见时自动更新；否则显示不在线，可修改地址 |
| 后台重连增加流量 | 只针对有可用来自共享助理的主人，30 秒一次，单地址连接 |
| 冒充设备 | 首次核对设备码、之后固定公钥；主人仍须当场确认每个申请 |

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/10-lan-foundation.md`、`11-share-and-request.md`、`12-use-shared-assistants.md`
- `src/crates/services/services-integrations/AGENTS.md`、`src/crates/assembly/core/AGENTS.md`、`src/apps/desktop/AGENTS.md`、`src/web-ui/AGENTS.md`
