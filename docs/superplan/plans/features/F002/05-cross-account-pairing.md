---
id: "F002-05"
title: "跨账号配对、连接级密钥与连接管理"
type: "feature"
status: "draft"
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

- [ ] 配对、指纹、公钥固定、密钥派生、撤销与损坏记录都有测试

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

- [ ] 加解密往返、撤销后拒绝、relay 能力缺失都有测试

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

- [ ] 入口测试通过，两个账号的配对实测结果已记录

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/04-relay-cross-account-routing.md`
- `docs/architecture/remote-surface-contract.md`
- `src/crates/services/services-integrations/AGENTS.md`
