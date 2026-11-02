---
id: "F002-04"
title: "relay 跨账号连接登记与路由授权"
type: "feature"
status: "superseded"
summary: "relay 支持一次性跨账号邀请、双方确认的连接记录与撤销，只在有效连接的两台设备之间路由委派 RPC，并在信息接口声明 cross_account_routing_v1。"
source: "docs/superplan/human/features.md"
created: "2026-10-30"
order: 4
depends_on: []
parent: "F002"
---

# relay 跨账号连接登记与路由授权 Plan

> 已被 F002 RFC 第 6 版（局域网共享助理）取代，替代计划为 F002-10 至 F002-13（待 RFC 批准后制定）。本计划交付的成果中，Dispatch 执行与审批、外部任务以助理身份运行且不改写助理、受阻时如实告知与排队、会话保留期限，按 RFC D12 迁移进新流程，其余随第 6 版删除。

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

- [x] 邀请一次性、可过期、可撤回；连接需要双方确认、可撤销；隔离有测试

**Progress (2026-10-30):** 完成。
- 新表 `connection_invites` 与 `device_connections`：
  - 邀请只保存邀请码的 SHA-256；有效期默认 1 小时，可设 60 秒到 7 天。
  - 连接记录在双方加入时固定各自设备公钥，并保存账号展示名与设备名。
  - 设备被删除时，触发器把它的连接置为 `revoked`（原因 `device_removed`），不删除记录。
- HTTP 接口：
  - 邀请：`POST /api/connections/invites` 创建，`GET` 列出本账号未用的邀请，`DELETE …/invites/{id}` 撤回。
  - 预览与接受：`POST …/invites/preview`、`POST /api/connections/accept`。
  - 连接：`GET /api/connections`，`POST …/{id}/confirm` 确认，`DELETE …/{id}` 撤销。
- 规则：
  - 只接受设备凭据；委托的控制凭据返回 `403 device_credential_required`。
  - 只有发出邀请的那台设备能确认；两个账号的任意设备都能撤销。
  - 第三个账号读、确认、撤销一律返回 `404 connection_not_found`；持有邀请码才能预览或接受。
  - 同账号接受返回 `400 same_account`；同两台设备已有未撤销的连接时返回 `409 connection_exists`。
  - 设备没有公钥时不能邀请或接受（`409 device_public_key_missing`）。
  - 每个账号最多 16 个未用邀请、256 个未撤销连接。
  - 错误都带 `reason`，邀请失效分别为 `410 invite_used`、`invite_withdrawn`、`invite_expired`。
- 接受、确认与撤销会给两端设备发 `connection-updated` 提示；这只是刷新提示，客户端仍以列表为准。
- 准入：`/api/connections` 与 `/api/devices` 一样，在读取请求体之前认证并按账号限流，请求体上限 16 KiB。
- Evidence：`cargo +1.95.0 test --locked -p openbitfun-relay-service --lib connections`：8 passed。`node scripts/check-core-boundaries.mjs`：通过。

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

- [x] 路由授权、撤销即时生效、白名单与同账号回归都有测试

**Progress (2026-10-30):** 完成。
- 新的 Socket.IO 事件 `connection-rpc-call`，参数为 `connectionId`、`command`、`params`（端到端加密，relay 不读）。
- 转发条件：
  - 只接受设备（machine-scoped）连接的调用。
  - 只在连接的两台设备之间转发，只转发列出的 16 个 `dispatch_target_*` 委派命令。
  - 目标必须注册 `<device>:connection-invoke`。旧客户端不会注册，因此跨账号请求不会进入它们的同账号处理入口。
  - 版本兼容与内存准入沿用 `rpc-call`。
- 转发给目标的 `rpc-request` 额外带 `command`、`connectionId` 与 `sourceAccountId`。F002-05 的目标端需要核对解密后的命令与 `command` 一致。
- 撤销即时生效：
  - 调用到达时、转发前一刻、回答返回前各检查一次连接。
  - 在 relay 中等待目标上线的调用，撤销后不再转发。
  - 目标处理期间被撤销的，回答不再返回给发起方。
- 拒绝原因可区分：`connection_not_found`、`connection_pending`、`connection_revoked`、`connection_key_changed`（配对后设备换了公钥）、`command_not_routable`、`target_unavailable`、`device_required`。
- 大载荷：经连接转发的载荷引用会授权给另一端账号下载；发起方不能读取的引用不转发。
- 能力声明：`/api/info` 声明 `cross_account_routing_v1`。
- 同账号 `rpc-call` 与设备路由未改动，原有测试全部通过。
- 偏差：
  - 计划列出的 `routes/devices.rs` 未改动：对方公钥由连接记录提供，无需放开跨账号公钥查询。
  - 白名单检查放在实时层（`realtime/mod.rs`），而不是 `admission.rs`：跨账号调用走 Socket.IO，不经过 HTTP 准入。
  - relay-server README 补充了接口与规则说明。
- 远程场景：本任务只改 relay，验证在进程内 relay 与真实 Socket.IO 连接上进行；跨机器与真实客户端要等 F002-05 的客户端实现后才能实测。
- Evidence：
  - `cargo +1.95.0 test --locked -p openbitfun-relay-service --lib account_transport_tests::`：9 passed，含 4 个新测试（双向路由与白名单、第三账号与 controller 被拒；待确认、未注册、撤销的区分；撤销对在途调用和迟到回答生效；大载荷只对另一端可读）。
  - 去掉转发前与返回前的两次复查时，撤销测试失败。
  - `cargo +1.95.0 test --locked -p openbitfun-relay-service`：137 passed。
  - `node scripts/check-core-boundaries.mjs`：通过。
  - `cargo +nightly clippy -p openbitfun-relay-service --tests`：新代码无告警（1.95.0 工具链未安装 clippy）。

## References
- `docs/superplan/rfcs/F002.md`
- `src/crates/services/relay-service/AGENTS.md`
