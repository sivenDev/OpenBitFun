---
id: "F002-04"
title: "relay 跨账号连接登记与路由授权"
type: "feature"
status: "approved"
summary: "relay 支持一次性跨账号邀请、双方确认的连接记录与撤销，只在有效连接的两台设备之间路由委派 RPC，并在信息接口声明 cross_account_routing_v1。"
source: "docs/superplan/human/features.md"
created: "2026-10-30"
order: 4
depends_on: []
parent: "F002"
---

# relay 跨账号连接登记与路由授权 Plan

**Goal:** relay 能安全地在不同账号的两台设备之间转发委派请求，且只转发给双方都确认过、尚未撤销的连接。

**Scope:**
- **邀请**：发起一方的设备创建一次性邀请，带过期时间，只记录邀请方的账号、设备、设备公钥和展示信息。邀请可以撤回；过期或已使用后不能再接受。
- **接受与确认**：
  - 另一账号的设备凭自己的登录接受邀请，relay 记录待确认连接并通知邀请方设备。
  - 邀请方设备确认后连接生效。
  - 双方各自查询连接列表时，都能看到对方的账号展示名、设备与公钥。
- **撤销**：任一方都可撤销连接，撤销立即生效：新的路由请求被拒绝，在途请求不再转发。
- **路由授权**：新增跨账号设备 RPC 路由，只在有效连接的两台设备之间、只为委派相关命令转发。载荷保持不透明，relay 不存储委派内容。同账号路由的行为不变。
- **能力声明**：relay 在信息接口声明 `cross_account_routing_v1`；客户端据此决定是否显示跨账号入口。

**Non-Goals:**
- 不做客户端配对界面、连接级密钥与本地连接存储（F002-05）。
- 不做开放项的跨账号授权、审批、额度与审计（F002-06）。
- 不做离线存储转发或 relay 侧排队。
- 不改变现有同账号设备、Peer、手机端路由。

**Architecture:**
- 连接与邀请是 relay 数据库中新增的表，按身份隔离：只有连接的两端设备所属账号能读到该连接。身份一律来自认证凭据，不采信请求中自报的账号或设备。
- 跨账号路由只接受白名单内的命令前缀（`dispatch_target_*` 中与委派相关的动词），其他命令一律拒绝。
- 准入在读取请求体之前完成，沿用现有的资源许可与配额边界。
- 独立部署与内嵌宿主使用同一套路由（遵循 relay-service 的 AGENTS 规则）。

**Baseline:**
- `devices` 表以 `(user_id, device_id)` 为主键，已经保存设备公钥；设备 RPC 路由要求调用方与目标属于同一用户（`routes/devices.rs`）。
- relay 只转发、不存储用户内容，唯一持久化的请求体是短期的加密 RPC 载荷引用。
- 账号隔离、设备 RPC 与撤销测试在 `account_transport_tests.rs`。

**Exit Criteria:**
- 有效连接的两台设备之间可以互相路由委派命令，其他命令被拒绝。
- 没有连接、连接待确认、连接已撤销时，跨账号路由被拒绝，错误可区分。
- 邀请只能使用一次，过期后不能接受，邀请方可以撤回。
- 第三个账号无法读取、接受（除非持有邀请）、撤销或路由该连接。
- 撤销后，即使请求已经到达 relay，也不再转发给对方。
- 旧客户端的同账号流程不受影响；relay 信息接口声明 `cross_account_routing_v1`。

## Task 1: 邀请与连接记录

**Outcome:** relay 支持创建、查询、接受、确认、撤回与撤销，数据按身份隔离。

**Files:**
- Modify: `src/crates/services/relay-service/src/db.rs`
- Create: `src/crates/services/relay-service/src/connections.rs`
- Modify: `src/crates/services/relay-service/src/routes/api.rs`
- Modify: `src/crates/services/relay-service/src/lib.rs`
- Test: `src/crates/services/relay-service/src/connections.rs`

**Verification:**
- `cargo +1.95.0 test -p openbitfun-relay-service --lib connections`
- `node scripts/check-core-boundaries.mjs`

- [ ] 邀请一次性、可过期、可撤回；连接需要双方确认、可撤销；隔离有测试

## Task 2: 跨账号路由授权与能力声明

**Outcome:** 跨账号设备 RPC 只在有效连接间、只为白名单命令转发，relay 声明能力。

**Files:**
- Modify: `src/crates/services/relay-service/src/routes/devices.rs`
- Modify: `src/crates/services/relay-service/src/realtime/payloads.rs`
- Modify: `src/crates/services/relay-service/src/admission.rs`
- Modify: `src/crates/services/relay-service/src/routes/api.rs`
- Test: `src/crates/services/relay-service/src/account_transport_tests.rs`

**Verification:**
- `cargo +1.95.0 test -p openbitfun-relay-service --lib account_transport_tests::`
- `cargo +1.95.0 test -p openbitfun-relay-service`

- [ ] 路由授权、撤销即时生效、白名单与同账号回归都有测试

## References
- `docs/superplan/rfcs/F002.md`
- `src/crates/services/relay-service/AGENTS.md`
