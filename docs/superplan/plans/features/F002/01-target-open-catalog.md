---
id: "F002-01"
title: "目标端开放智能体清单与纯消息任务"
type: "feature"
status: "in_progress"
summary: "目标设备主人可以把指定原生智能体开放给同账号的其他设备；目标端声明 agent_delegation_v1，提供按调用设备过滤的目录查询，并能执行不经 Git 的纯消息委派任务。"
source: "docs/superplan/human/features.md"
created: "2026-10-30"
order: 1
depends_on: []
parent: "F002"
---

# 目标端开放智能体清单与纯消息任务 Plan

**Goal:** 目标设备能声明「对哪些设备开放哪些智能体」，并安全地接收、执行委派任务，为发起方（F002-02）提供可依赖的目标端契约。

**Scope:**
- **开放清单**：持久化在目标端本机（`<product_home>/delegation/open-agents.json`），默认为空。每个开放项包括：稳定 id、智能体类型（本计划仅限原生智能体，`acp:` 条目在 F002-03 开放）、展示名、说明、允许的任务类型（纯消息、编码）、纯消息任务的工作区绑定（可选），以及同账号授权（全部设备或指定设备 id）。
- **管理入口**：桌面设置页新增「对外开放的智能体」；CLI 新增 `openbitfun delegation list|open|close`，供无界面主机（CLI Peer Host、账号守护进程）使用。
- **能力声明**：新增 `agent_delegation_v1`，只在本机支持开放清单与委派提交时声明。
- **目录查询**：新增目标端动词 `catalog`（设备 RPC 名为 `dispatch_target_catalog`），只返回对调用设备授权的开放项。调用设备取自设备 RPC 的来源设备，由宿主注入，不采信请求载荷中的自报值。
- **委派提交**：`submit` 新增可选字段 `delegation`，包括开放项 id、任务类型、来源元数据（账号、设备、会话、智能体）。目标端校验开放项存在、已授权给调用设备、允许该任务类型，并由开放项决定实际智能体类型；不一致时拒绝。
- **纯消息任务**：不走 Git。在开放项绑定的工作区运行；未绑定时，在目标端为「开放项 × 来源设备」新建独立的空工作区。发起方不能指定目标机路径。结果为最终回答，工作区内的文件通过现有的文件查询能力按需读取，有大小上限。
- **编码任务**：沿用 Dispatch 的 Git worktree 交付，只是增加上面的开放项校验。
- **委派链与来源**：委派任务的会话不提供远程委派能力；来源元数据写入任务记录并展示为「来自某设备的智能体请求」；委派内容按不可信输入处理。

**Non-Goals:**
- 不做发起方的列表、委派与结果回收（F002-02）。
- 不开放外部智能体（F002-03）。
- 不做跨账号连接、授权、额度与审计（F002-04 至 F002-06）。
- 不改变现有 Dispatch v6 动词的语义；不加入 relay 侧排队。

**Architecture:**
- 目标端 CLI 仍是委派任务、会话、事件日志与权限邮箱的唯一所有者；桌面与 CLI Peer Host 只负责把 `dispatch_target_catalog` 和带委派的 `submit` 路由到 CLI。
- 宿主在路由前写入保留参数（暂定 `__sourceDeviceId`），并丢弃请求中同名的输入。目标端只信任这个保留值。
- 能力名只在 `dispatch_contract.rs` 定义，Web UI 合约测试同步固定。
- 开放清单是目标端本机文件（不进入可导出的配置文档），读取宽容；无法解析的开放项保留在文件中并视为未开放，同时显示原因。
- 委派任务的审批策略固定为 `remote`；同账号时由账号主人从任一驱动界面回答。
- 管理开放清单的新增桌面命令在产品操作注册表中各占一行，不增加 `Unaudited`。

**Baseline:**
- 目标端动词由 `src/apps/desktop/src/api/dispatch_host.rs` 的 `target_cli_verb` 和 `src/apps/cli/src/peer_host/dispatch.rs` 路由到 `openbitfun dispatch <verb>`；`dispatch_host::dispatch` 目前不接收来源设备。
- 来源设备在 `remote_connect_api.rs` 的设备 RPC 处理中已经可得（`source_device_id`）。
- 能力清单在 `services-core/src/dispatch_contract.rs`；`probe` 在 `src/apps/cli/src/dispatch/mod.rs` 汇总能力。
- `DispatchSubmitRequest`（CLI 端 `dispatch/protocol.rs`）使用 `deny_unknown_fields`，因此旧目标收到 `delegation` 字段会拒绝提交；发起方必须先确认 `agent_delegation_v1`。
- Dispatch 交付目前只支持 Git；`AskUserQuestion` 在 Dispatch 中明确失败。

**Exit Criteria:**
- 未开放任何智能体时，`catalog` 返回空列表，不声明额外条目。
- 开放给设备 A 的条目在设备 A 查询时可见，设备 B 不可见；伪造的来源设备字段不改变结果。
- 带委派的纯消息任务在绑定工作区或新建的独立空工作区中运行，结束后能读取最终回答；请求中携带的目标路径被忽略或拒绝。
- 未授权、开放项不存在、任务类型不允许、智能体类型不一致时，提交明确失败并给出原因，不创建任务。
- 委派任务内看不到、也无法调用远程委派。
- 产品操作注册表与能力清单检查通过，`Unaudited` 数量不增加。

## Task 1: 开放清单配置与管理入口

**Outcome:** 主人可以在桌面设置页或 CLI 中查看、开放、关闭智能体，配置读写对旧数据宽容。

**Files:**
- Create: `src/crates/assembly/core/src/service/delegation.rs`
- Modify: `src/crates/assembly/core/src/service/mod.rs`
- Modify: `src/apps/cli/src/main.rs`
- Create: `src/apps/cli/src/delegation_cli.rs`
- Create: `src/apps/desktop/src/api/delegation_api.rs`
- Modify: `src/apps/desktop/src/api/mod.rs`, `src/apps/desktop/src/lib.rs`
- Modify: `src/crates/contracts/product-domains/src/remote_surface/table.rs`
- Modify: `src/shared/interactive-capabilities/catalog.json`（及其生成物）
- Create: `src/web-ui/src/infrastructure/api/service-api/DelegationAPI.ts`
- Create: `src/web-ui/src/infrastructure/config/components/AgentDelegationConfig.tsx`
- Modify: `src/web-ui/src/app/scenes/settings/pages/ai/PermissionsSettingsPage.tsx`
- Modify: `src/web-ui/src/locales/*/settings/runtime.json`
- Test: `src/crates/assembly/core/src/service/delegation.rs`、`src/web-ui/src/infrastructure/config/components/AgentDelegationConfig.test.tsx`

**Change Map:**
- `delegation.rs`: 开放项类型、本机存储、读取与校验、按来源设备过滤的授权判断
- `delegation_cli.rs` / `delegation_api.rs`: 管理入口，两者调用同一核心实现（`OpenAgentStore::open_local_agent`）
- `table.rs`: 新增命令的远程面立场
- `AgentDelegationConfig.tsx`: 权限设置页中的「对外开放的智能体」分区

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib service::delegation`
- `cargo +1.95.0 test -p openbitfun-product-domains --no-default-features remote_surface`
- `pnpm run capabilities:generate && pnpm run capabilities:check`
- `pnpm run i18n:audit`
- `pnpm run type-check:web`

- [x] 开放清单可通过桌面与 CLI 管理，旧配置与无法解析的开放项按约定处理

**Deviations:**
- 存储位置：计划写的是「持久化在目标端配置中」（`config/types.rs`）。实际改为本机文件 `<product_home>/delegation/open-agents.json`，因为配置文档可以导出/导入，会把授权带到另一台设备。
- 模块路径：`service/dispatch/` 受 `dispatch-store` 特性约束，而 core-boundaries 禁止 CLI 闭包启用该特性，所以模块放在 `service/delegation.rs`（不受特性约束；校验本机智能体的 `open_local_agent` 需要 `agent-runtime`）。
- 设置入口：没有新增独立设置页，而是作为分区放在「权限」页，因为开放属于授权决定；能力目录中归入 `feature.detached-dispatch`。
- 远程立场：三个桌面命令均为 `Agnostic` / `ControllerLocal`。Peer 模式下界面显示「请在该设备上管理」，不代管对端清单；非桌面宿主显示不支持。

**Evidence (2026-10-29):**
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib service::delegation`：6 passed（默认关闭、按设备与任务类型授权、重开替换与关闭、非法项拒绝、无法解析项保留并可关闭、损坏文件不被覆盖、旧智能体 id 规范化）。
- `cargo +1.95.0 check --locked -p openbitfun-core --no-default-features`、`cargo +1.95.0 check --locked -p openbitfun-desktop`：通过。
- `cargo +1.95.0 test --locked -p openbitfun-cli --bin openbitfun dispatch_commands_parse_and_internal_worker_is_hidden`：通过（未指定设备或同时指定设备与全部设备时解析失败）。
- `cargo +1.95.0 test -p openbitfun-product-domains --no-default-features remote_surface`：20 passed；`cargo +1.95.0 test --locked -p openbitfun-desktop --lib remote_workspace_policy`：9 passed。
- `pnpm run capabilities:generate && pnpm run capabilities:check`、`pnpm run check:core-boundaries`、`pnpm run i18n:audit`（0 warning）、`pnpm run check:web`：通过。
- `pnpm --dir src/web-ui run test:run src/infrastructure/config/components/AgentDelegationConfig.test.tsx`：4 passed；设置场景聚焦测试 44 files / 279 tests passed。
- 手动：在隔离的 `OPENBITFUN_HOME`（`/tmp`，700，已删除）中执行 `openbitfun delegation list|open|close`：空列表、未知智能体拒绝、旧 id `agentic` 规范化为 `Standard`、重复任务类型去重、无法解析项显示为已关闭且可按 id 关闭。
- 远程场景：仅本机验证；Peer 模式与非桌面宿主的不可用状态只由 DOM 测试覆盖。

## Task 2: 能力声明与目录查询

**Outcome:** 目标端声明 `agent_delegation_v1`，`dispatch_target_catalog` 只返回对调用设备授权的开放项，来源设备由宿主注入。

**Files:**
- Modify: `src/crates/services/services-core/src/dispatch_contract.rs`
- Modify: `src/apps/cli/src/dispatch/mod.rs`
- Modify: `src/apps/cli/src/dispatch/protocol.rs`
- Modify: `src/apps/desktop/src/api/dispatch_host.rs`
- Modify: `src/apps/desktop/src/api/remote_connect_api.rs`
- Modify: `src/apps/cli/src/peer_host/dispatch.rs`
- Modify: `src/web-ui/src/features/dispatch/dispatch.contract.test.ts`
- Test: `src/apps/cli/src/dispatch/mod.rs`

**Change Map:**
- `dispatch_contract.rs`: 能力常量
- `dispatch/mod.rs`: `catalog` 动词与 `probe` 能力
- `dispatch_host.rs` / `peer_host/dispatch.rs`: 注入保留的来源设备参数，丢弃同名输入

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `cargo +1.95.0 test --locked -p openbitfun-cli --bin openbitfun peer_host::`
- `pnpm --dir src/web-ui run test:run src/features/dispatch/dispatch.contract.test.ts`
- `cargo +1.95.0 check -p openbitfun-desktop`

- [ ] 目录按调用设备过滤，伪造来源无效，旧目标不声明该能力

## Task 3: 委派提交校验与纯消息任务执行

**Outcome:** 带委派的提交经开放项校验后创建任务；纯消息任务在受控工作区运行，委派任务内不提供远程委派。

**Files:**
- Modify: `src/apps/cli/src/dispatch/protocol.rs`
- Modify: `src/apps/cli/src/dispatch/mod.rs`
- Modify: `src/apps/cli/src/dispatch/workspace.rs`
- Modify: `src/apps/cli/src/dispatch/worker.rs`
- Modify: `src/apps/cli/src/dispatch/store.rs`
- Test: `src/apps/cli/src/dispatch/mod.rs`

**Change Map:**
- `protocol.rs`: 可选 `delegation` 字段（新字段带默认值）
- `workspace.rs`: 纯消息任务工作区的解析与创建，不接受发起方路径
- `worker.rs`: 委派任务关闭远程委派能力并写入来源元数据
- `store.rs`: 任务记录携带来源元数据

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- 手动：在本机用 `openbitfun dispatch submit` 提交带委派的纯消息任务，确认工作区位置、回答可读、未授权时明确失败

- [ ] 委派提交的校验、纯消息工作区与禁止多跳都有测试覆盖，并在本机实测

## References
- `docs/superplan/rfcs/F002.md`
- `docs/architecture/detached-task-dispatch.md`
- `docs/architecture/remote-surface-contract.md`
- `src/apps/cli/AGENTS.md`
