---
id: "F002-05"
title: "跨账号配对、连接级密钥与连接管理"
type: "feature"
status: "complete"
summary: "设备之间通过一次性邀请配对，接受前核对指纹；连接级密钥由双方设备身份密钥派生，跨账号委派 RPC 用它端到端加密；双方都能查看与撤销连接。"
source: "docs/superplan/human/features.md"
created: "2026-10-30"
order: 5
depends_on: ["F002-04"]
parent: "F002"
---

# 跨账号配对、连接级密钥与连接管理 Plan

**Goal:** 两个不同账号的设备能建立一条可核验、可撤销的连接，并通过它安全地传输委派请求，账号主密钥始终不离开本账号。

**Scope:**
- **配对**：
  - 一方生成邀请链接或二维码，带过期时间。
  - 另一方打开邀请后看到对方的账号展示名、设备和指纹，核对后才能接受。
  - 邀请方确认后连接生效。
- **设备身份**：配对使用设备身份公钥并在本地固定对方公钥。之后对方公钥发生变化时，连接停用并提示重新配对，不自动接受新公钥。
- **连接级密钥**：由双方设备身份密钥协商派生，并绑定连接 id。跨账号的委派请求与响应用它端到端加密，账号主密钥不参与。
- **连接存储**：双方本地持久化连接（连接 id、对方账号与设备、对方公钥、状态、建立时间）。无法解析的记录保留并停用，同时显示原因，不删除。
- **连接管理**：
  - 桌面设置页新增「连接」：生成邀请、接受邀请、查看连接、撤销。
  - CLI 新增 `openbitfun connections invite|accept|list|revoke`，供无界面主机使用。
- **传输**：跨账号目标的设备 Dispatch RPC 经连接级密钥加密后走 F002-04 的跨账号路由；relay 未声明 `cross_account_routing_v1` 时，跨账号入口显示为不可用并说明原因。

**Non-Goals:**
- 不做开放项的跨账号授权、目录、审批、额度与审计（F002-06）。
- 不改变同账号的主密钥加密与设备通道。
- 不做 WebRTC 直连。

**Architecture:**
- 连接与密钥逻辑放在服务层的 remote_connect 模块中，与账号设备通道并列。桌面和 CLI 只提供入口与展示。
- 跨账号传输实现现有的 `DeviceDispatchRpc` 接口（以连接为目标），因此 F002-06 可以直接复用设备 Dispatch 控制器。
- 私钥不离开设备，日志不记录密钥材料、邀请密文或指纹以外的身份数据。
- 新增桌面命令在产品操作注册表中各占一行。账号身份与配对属于本机操作，按注册表规则声明为 `ControllerLocal`。

**Baseline:**
- 账号设备载荷用账号主密钥派生的消息密钥加密（`services-integrations/src/remote_connect/account.rs`，`device_crypto::derive_message_key`）。
- relay 已经保存每台设备的公钥。
- 桌面经 `remote_connect_api::account_device_rpc` 发起设备 RPC；CLI 有账号路由运行时（`src/apps/cli/src/account.rs`）。

**Exit Criteria:**
- 两个不同账号的设备通过邀请完成配对，接受前显示的指纹与邀请方设备一致。
- 配对后，双方的连接列表都显示对方，跨账号 RPC 使用连接级密钥加密；relay 侧只看到密文。
- 篡改公钥或使用他人的邀请时配对失败；过期邀请无法接受。
- 任一方撤销后，双方的连接都显示已撤销，新的跨账号请求被拒绝。
- relay 不支持跨账号路由时，入口不可用并给出原因。
- 损坏的连接记录被保留并停用，应用照常启动。

## Task 1: 设备身份、连接级密钥与连接存储

**Outcome:** 服务层提供配对协议、指纹、密钥派生与本地连接存储，对旧数据和损坏记录宽容。

**Files:**
- Create: `src/crates/services/services-integrations/src/remote_connect/connections.rs`
- Modify: `src/crates/services/services-integrations/src/remote_connect/mod.rs`
- Modify: `src/crates/services/services-integrations/src/remote_persistence.rs`
- Test: `src/crates/services/services-integrations/src/remote_connect/connections.rs`

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-services-integrations --lib remote_connect::connections`

- [x] 配对、指纹、公钥固定、密钥派生、撤销与损坏记录都有测试

- 结果：
  - 指纹：设备公钥 SHA-256 的前 16 字节，分 8 组、每组 4 位十六进制。比较时忽略空格与大小写。
  - 邀请链接：`openbitfun://connect?relay=…&code=…&fp=…`，带邀请方指纹。接受方先把 relay 报告的邀请方公钥与链接里的指纹比对，不一致即失败（`fingerprint_mismatch`）。因此替换公钥的 relay 无法冒充邀请方。邀请来自另一个 relay 时直接拒绝（`different_relay`）。
  - 公钥固定：接受方在接受时固定邀请方公钥；邀请方在确认时核对人工比对过的指纹，再固定对方公钥。之后 relay 报告的公钥不同，或 relay 返回 `connection_key_changed` 时，连接停用（`key_changed`），不接受新公钥。
  - 连接级密钥：设备 X25519 密钥协商后经 HKDF-SHA256 派生。salt 与同账号设备密钥不同，info 为排序后的两把公钥加连接 id，每条连接一把密钥，账号主密钥不参与。
  - 本地存储：`relay-v1.0.0/connections.json`（与账号会话同目录，权限仅本人）。无法解析的条目原样保留并显示为停用；整个文件无法解析时不覆盖。新增字段都有默认值，旧记录可读，未知字段忽略。
  - 请求信封：密文内也带连接 id 与命令，目标端核对与 relay 报告的路由一致。
- 偏差：
  - 模块入口是 `remote_connect.rs`，计划写的是 `remote_connect/mod.rs`。
  - 计划中的验证命令缺少 feature，实际命令为 `--no-default-features --features remote-connect`。
- Evidence：`cargo +1.95.0 test --locked -p openbitfun-services-integrations --no-default-features --features remote-connect --lib remote_connect::connections`：9 passed，含替换邀请方公钥的假 relay 被拒。remote_connect 全部 183 个测试通过。

## Task 2: 跨账号传输适配器

**Outcome:** 以连接为目标的设备 Dispatch RPC 经连接级密钥加密后走跨账号路由，relay 能力缺失时明确不可用。

**Files:**
- Modify: `src/crates/assembly/core/src/service/remote_connect/account_runtime.rs`
- Create: `src/crates/assembly/core/src/service/remote_connect/connection_rpc.rs`
- Modify: `src/apps/desktop/src/api/remote_connect_api.rs`
- Modify: `src/apps/cli/src/account.rs`
- Test: `src/crates/assembly/core/src/service/remote_connect/connection_rpc.rs`

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib service::remote_connect::connection_rpc`
- `cargo +1.95.0 check -p openbitfun-desktop`

- [x] 加解密往返、撤销后拒绝、relay 能力缺失都有测试

- 结果：
  - 发起方：`ConnectionRpc` 以连接 id 为目标实现 `DeviceDispatchRpc`。它用连接级密钥加密请求，经设备自身的路由连接发出 `connection-rpc-call`（relay 只接受设备连接发出的跨账号调用）。
  - 发起方先检查三件事，任一不满足就不发送：relay 声明了 `cross_account_routing_v1`（否则 `relay_unsupported`）、命令在委派命令白名单内、本地连接可用。relay 拒绝为 `connection_revoked` 或 `connection_key_changed` 时，本地记录同步标为已撤销或已停用。
  - 目标端：设备路由连接额外注册 `<device>:connection-invoke`；旧 relay 不接受这个注册也不影响原有连接。收到调用后，目标端先按本地固定记录核对连接 id、来源账号与设备，再用连接密钥解密，并核对密文内的连接 id 与命令，然后才交给处理器。
  - 默认处理器：在 F002-06 的逐项授权之前一律拒绝，拒绝信息在加密的应答里返回（“cross-account delegation is not enabled on this device”）。目标端没有可用密钥时（未知、已撤销或密钥变化）只返回拒绝原因，不返回内容。
  - 桌面与 CLI 的设备路由循环都接入了目标端处理。`AccountRuntime::connection_context` 提供会话、relay 与经认证的设备 id。
- 偏差：
  - 计划中的验证命令缺少 feature，实际命令为 `--no-default-features --features remote-connect`。
  - `DeviceDispatchRpc` 的实现与该 trait 一样，受 `dispatch-store` 与 `ssh-remote` 两个 feature 控制。
  - 另外修改了 services 的 `realtime_client.rs` 与 `relay_client.rs`（`connection-rpc-call`、`RelayEvent::ConnectionRequest`），以及 core `remote_connect/mod.rs`（暴露设备路由连接）。
- Evidence：
  - `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features remote-connect --lib service::remote_connect::connection_rpc`：8 passed。覆盖往返且 relay 只见密文、默认拒绝、relay 能力缺失时不发送、非委派命令不出本机、relay 撤销后记录同步且之后不再发送、密钥变化停用、relay 改写命令或冒充来源被目标端拒绝、目标端本地撤销后无密钥拒绝。
  - `cargo +1.95.0 check -p openbitfun-desktop -p openbitfun-cli`：通过。
  - 同账号委派模拟回归：`delegation_simulation::same_account_devices_take_delegated_jobs_and_other_accounts_cannot_reach_them` 通过。

## Task 3: 连接管理入口与实测

**Outcome:** 桌面与 CLI 都能生成邀请、接受邀请、查看与撤销连接，并用两个账号实测通过。

**Files:**
- Create: `src/apps/desktop/src/api/connections_api.rs`
- Create: `src/apps/cli/src/connections_cli.rs`
- Modify: `src/apps/cli/src/main.rs`
- Modify: `src/crates/contracts/product-domains/src/remote_surface/table.rs`
- Create: `src/web-ui/src/infrastructure/config/components/ConnectionsConfig.tsx`
- Modify: `src/web-ui/src/locales/*/` 相关文案
- Modify: `docs/superplan/plans/features/F002/05-cross-account-pairing.md`
- Test: 对应组件的 Vitest 测试

**Verification:**
- `cargo +1.95.0 test -p openbitfun-product-domains --no-default-features remote_surface`
- `pnpm run capabilities:generate && pnpm run capabilities:check`
- `pnpm --dir src/web-ui run test:run <changed test files>`
- `pnpm run i18n:audit`
- 手动：两个账号的设备完成配对、核对指纹、撤销；结果记录在本计划中

- [x] 入口测试通过，两个账号的配对实测结果已记录

- 结果：
  - 桌面：设置「权限规则」页新增「与其他账号的连接」区块，与「对其他设备开放的智能体」相邻。可以查看本设备指纹、创建与收回邀请、接受邀请（先看到邀请方并自动核对链接里的指纹）、核对并确认、撤销。损坏的记录显示为已停用，同时给出原因。
  - 桌面在以下情况明确显示入口不可用：relay 不支持跨账号路由、未登录、非桌面环境、Peer Device 模式。错误按原因码显示对应文案。
  - 桌面命令共 7 个（`connections_overview`、`connections_create_invite`、`connections_preview_invite`、`connections_accept_invite`、`connections_confirm`、`connections_revoke`、`connections_withdraw_invite`），在注册表中均为 `Agnostic` + `ControllerLocal`。能力目录新增 `feature.remote-connect` 下的 `connections` 条目。
  - CLI：`openbitfun connections invite|accept|confirm|list|revoke`。`accept` 不带 `--yes` 时只显示邀请方与指纹；`invite`、`accept` 与 `list` 支持 `--json`。
  - 撤销后两端都显示「已撤销」，同两台设备可以重新配对。
- 偏差：
  - 增加了 `confirm` 子命令，邀请方用它完成配对。
  - 设置页挂在已有的「权限规则」页中，没有新增独立页面。
  - 另外修改了 `src/shared/interactive-capabilities/catalog.json` 及其生成文件，并新增 `ConnectionsAPI.ts`。
- 修复（实测中发现）：
  - 接受预览的过期时间占位符写错，页面显示 `{{time}}`。测试现在会按英文文案核对键与占位符，改回错误写法时测试失败。
  - relay 再次列出某条损坏的记录时，会生成一条未固定公钥的重复记录。现在损坏记录保留原 id，不再被覆盖。
  - 连接文件整体损坏时：总览不再误报「无法连接 relay」；接受与确认在请求 relay 之前就拒绝（`store_unreadable`），邀请不会被消耗。
- 实测（本机模拟两个账号，`scripts/dev/sim-devices.mjs`：A1 为调试版桌面，账号 A；B1 为真实 `daemon run`，账号 B，用 CLI 操作）：
  - A1 在设置页创建邀请。B1 `connections accept` 显示的邀请方指纹与 A1 设置页显示的指纹一致。接受后 A1 显示「等待你确认」，核对面板中 B1 的指纹与 B1 自报的一致。确认后两端都显示「已生效」。A1 撤销后，B1 显示 `revoked`。
  - 反方向：B1 用 CLI 创建邀请。A1 粘贴改过指纹的链接时提示「设备密钥与指纹不一致」，不接受；粘贴原链接时预览显示 B1 与正确指纹，接受后 B1 显示待确认。B1 用错误指纹确认被拒，用正确指纹确认后连接生效；B1 撤销后，A1 显示「已撤销」。
  - 连接文件权限为 `-rw-------`。把一条记录的字段改坏后，页面显示该记录已停用并给出原因。把整个文件写成非 JSON 后，重启桌面应用照常启动，页面显示「连接文件」已停用，文件内容保持不变；此时接受邀请被拒，B1 的邀请仍未使用。恢复文件后，用同一邀请接受、确认、撤销都成功。
  - 模拟结束后没有残留进程或临时目录。
- 远程场景：
  - 连接管理使用本机账号身份，Peer Device 模式下入口不可用（注册表 `ControllerLocal`，页面显示对应说明）。
  - 跨账号调用的目标端在 CLI daemon 上实测（见下方 CLI 测试）。桌面的目标端只接入了同一个核心处理函数并通过编译，没有作为目标实测。
  - 跨机器实测、旧版 relay 实测都未做：relay 能力缺失只有单元测试与页面测试覆盖。
- Evidence：
  - `cargo +1.95.0 test --locked -p openbitfun-cli --test cli_command_contracts delegation_simulation::`：2 passed。新测试 `devices_of_different_accounts_pair_and_call_over_a_connection` 在进程内 relay 与真实 B1 daemon 上走完 CLI 邀请、接受、单次邀请、错误指纹确认被拒、确认、双方列表、经连接的加密调用（B1 daemon 解密并在加密应答中拒绝）、撤销后 relay 拒绝且双方显示已撤销。同账号委派测试照常通过。
  - `cargo +1.95.0 test -p openbitfun-product-domains --no-default-features remote_surface`：20 passed。
  - `pnpm run capabilities:generate && pnpm run capabilities:check`：通过。
  - `pnpm --dir src/web-ui run test:run src/infrastructure/config/components/ConnectionsConfig.test.tsx`：9 passed。`peer-device-adapter.test.ts` 与 `AgentDelegationConfig.test.tsx`：82 passed。
  - `pnpm run type-check:web`：通过。`pnpm run lint:web`：新文件无告警。`pnpm run i18n:audit`：通过。
  - `pnpm run check:core-boundaries`：通过。

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/04-relay-cross-account-routing.md`
- `docs/architecture/remote-surface-contract.md`
- `src/crates/services/services-integrations/AGENTS.md`
