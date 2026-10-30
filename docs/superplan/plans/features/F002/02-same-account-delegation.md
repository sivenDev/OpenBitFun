---
id: "F002-02"
title: "同账号委派给远程原生智能体"
type: "feature"
status: "approved"
summary: "发起方的智能体列表显示同账号设备开放的远程智能体；助理用 AgentSpawn 后台委派纯消息或编码任务，后端观察器按游标收回结果交给 AgentWait，重启后可补收。"
source: "docs/superplan/human/features.md"
created: "2026-10-30"
order: 2
depends_on: ["F001-02", "F001-03", "F002-01"]
parent: "F002"
---

# 同账号委派给远程原生智能体 Plan

**Goal:** 助理能把任务交给同账号另一台设备上开放的智能体，不被阻塞，结果回到发起会话，发起方重启也不会丢失结果。

**Scope:**
- **寻址**：远程智能体的子智能体类型为 `remote:<deviceId>/<openItemId>`。
- **列表与目录**：
  - 发起方向同账号在线设备查询目录；只查询声明了 `agent_delegation_v1` 与现有必需能力的设备，带短期缓存与超时。
  - 智能体列表的远程条目显示所属设备、在线状态与兼容性，只读；离线或不兼容的条目显示原因。
  - `AgentSpawn` 的可选类型只包含可用的远程条目，并与 F001 外部智能体使用同一道准入检查：父会话有权限限制、处于 DeepReview 或 Swarm 时不提供。
- **委派**：
  - 远程委派只能后台运行。
  - 任务类型可选纯消息或编码，默认纯消息；编码任务要求发起会话在 Git 工作区中。
  - 子会话可打开，显示目标端投影过来的过程。
- **结果**：
  - 纯消息任务返回最终回答。
  - 编码任务结束时同步一次结果分支到发起方的托管基线 worktree，结果中给出分支名、提交和改动文件；不改动用户自己的工作目录。
- **取消**：`AgentControl` 中断或父回合取消时，向目标端发出 Dispatch `cancel`，子会话进入已取消状态。
- **审批**：目标端的待审批请求在子会话与父会话中显示为「等待目标设备审批」。同账号由账号主人从任一驱动界面回答，回答经 Dispatch `answer` 送达目标端。
- **重启补收**：带远程关联的后台任务在发起方重启后不标记为中断，而是重新挂接观察器；结果完成后照常交给 `AgentWait` 或在父会话下一轮交付。
- **明确失败**：以下情况都在提交前或观察中给出明确原因，不在本机执行替代：
  - 目标离线；
  - 目标缺少能力；
  - 目标拒绝（未授权、开放项已关闭）；
  - 发起方宿主不支持（CLI、Dispatch 工作进程、未登录账号）。

**Non-Goals:**
- 不做远程外部智能体（F002-03）。
- 不做跨账号（F002-04 至 F002-06）。
- 不做多跳委派，也不让远程子会话接收 `fork_context`。
- 不新增按次数的硬编码限制。

**Architecture:**
- **端口**：内核通过新的端口（暂定 `RemoteAgentDelegationPort`，放在 `runtime-ports`）使用宿主能力，包括：列出远程智能体、提交、读取状态与事件、取消、回答审批、同步编码结果。
  - 桌面宿主用现有的设备 Dispatch 控制器（`submit_device`、`status_device` 等）和账号设备 RPC 实现它。
  - 其他宿主不注册该端口，委派明确报不支持。
- **执行分支**：分支点与 F001 相同：协调器看到 `remote:` 前缀时走远程委派路径。后台记录、取消令牌、`AgentWait` 都沿用现有子智能体机制。
- **持久化**：后台任务记录新增可选的远程关联（目标设备、任务 id），出站记录沿用 Dispatch 现有格式，只增加可选的发起会话关联字段。旧记录照常读取。
- **观察器**：观察器在后端按游标运行，不依赖界面是否打开。Web UI 的 `DispatchJobObserver` 只用于手动提交的 Dispatch 任务，两者不重复观察同一任务。
- **远程场景**：
  - Peer 模式下，出站记录与观察器属于实际执行内核的对端主机。
  - 手机端与 IM 通过现有后台任务状态与权限邮箱看到委派和审批。
  - Dispatch 工作进程中远程智能体不可用，并明确报错。
  - 委派任务的回合上下文带 `dispatchDelegation`（F002-01 写入）；带该标记的会话不提供 `remote:` 条目，强行指定时明确拒绝（禁止多跳）。
  - 目标端的任务列表响应不含委派来源（旧控制端以 `deny_unknown_fields` 解析列表）；需要展示「来自某设备的请求」时，经能力协商后再读取。

**Baseline:**
- 设备 Dispatch 控制器在 `src/crates/assembly/core/src/service/dispatch/device_controller.rs`，经 `DeviceDispatchRpc` 调用目标端；只有桌面实现了该 RPC（`src/apps/desktop/src/api/dispatch_api.rs`）。
- 观察目前在 Web UI（`src/web-ui/src/features/dispatch/DispatchJobObserver.ts`，约 1.8 秒轮询一次）。
- 后台任务记录在 `coordination_store.rs`；新进程启动时，旧所有者的运行中任务会被标记为 `interrupted`。
- 已完成的后台结果能在重启后从子会话历史中恢复（`background_outcomes.rs`）。
- F001 的 `acp:` 列表合并与准入检查（`registry/acp_agents.rs`、`task/execution.rs`、`skill_agent_snapshot.rs`）可作为远程条目的参照。

**Exit Criteria:**
- 同账号设备开放智能体后，发起方智能体列表可见该远程智能体，并显示设备与在线状态；关闭开放后不再出现。
- 助理委派纯消息任务后立即继续，`AgentWait` 取回目标端的回答。
- 委派编码任务后，结果分支同步到发起方托管基线 worktree，结果中有分支名和改动文件，用户工作目录不变。
- 委派运行中关闭再重新打开发起方，结果仍能补收。
- 取消委派后，目标端任务进入已取消状态。
- 目标端审批请求在发起方显示，回答后目标任务继续。
- 目标离线、能力缺失、未授权时委派明确失败；CLI 中远程条目不出现，强行指定时明确报不支持。

## Task 1: 远程委派端口与智能体目录

**Outcome:** 发起方能列出同账号设备开放的远程智能体，管理列表与 `AgentSpawn` 目录按准入规则显示。

**Files:**
- Create: `src/crates/contracts/runtime-ports/src/remote_agent.rs`
- Modify: `src/crates/contracts/runtime-ports/src/lib.rs`
- Create: `src/crates/assembly/core/src/agentic/agents/registry/remote_agents.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/query.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/mod.rs`
- Modify: `src/crates/assembly/core/src/agentic/skill_agent_snapshot.rs`
- Create: `src/apps/desktop/src/api/remote_agent_delegation.rs`
- Modify: `src/apps/desktop/src/lib.rs`
- Test: `src/crates/contracts/runtime-ports/tests/remote_agent_port_contracts.rs`

**Change Map:**
- `remote_agent.rs`: 端口与 DTO（远程智能体描述、可用状态、任务类型）
- `remote_agents.rs`: `remote:` 条目投影，只读
- `task/mod.rs` / `skill_agent_snapshot.rs`: 与 `acp:` 共用准入检查
- `remote_agent_delegation.rs`: 桌面端目录查询（设备筛选、缓存、超时）

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-runtime-ports --features remote-agent-port --test remote_agent_port_contracts`
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::agents::registry`
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib skill_agent`

- [ ] 远程条目在列表与目录中按可用性和准入规则出现

## Task 2: 提交、观察、取消、审批与重启补收

**Outcome:** `remote:` 子智能体在后台运行，后端观察器把目标事件投影到子会话，终态结果交给 `AgentWait`，取消与审批可用，重启后补收。

**Files:**
- Create: `src/crates/assembly/core/src/agentic/coordination/remote_agent_turn.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordinator.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordination_store.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/background_outcomes.rs`
- Modify: `src/crates/assembly/core/src/service/dispatch/device_controller.rs`
- Modify: `src/apps/desktop/src/api/remote_agent_delegation.rs`
- Test: `src/crates/assembly/core/src/agentic/coordination/remote_agent_turn/tests.rs`

**Change Map:**
- `remote_agent_turn.rs`: 提交、游标观察、事件投影、终态结果、取消、审批转发
- `coordination_store.rs`: 后台任务的可选远程关联；启动时带远程关联的运行中任务不标记为中断
- `background_outcomes.rs`: 重新挂接后的结果交付

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib remote_agent`
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib coordination_store`
- 包含旧后台任务记录的反序列化与往返测试

- [ ] 后台委派、结果交付、取消、审批与重启补收都有内核测试覆盖

## Task 3: 编码任务的 Git 交付与结果同步

**Outcome:** 编码任务从发起会话的工作区建立托管基线并提交，结束时同步结果分支，结果包含分支与改动摘要。

**Files:**
- Modify: `src/crates/assembly/core/src/agentic/coordination/remote_agent_turn.rs`
- Modify: `src/crates/assembly/core/src/service/dispatch/baseline.rs`
- Modify: `src/apps/desktop/src/api/remote_agent_delegation.rs`
- Test: `src/crates/assembly/core/src/agentic/coordination/remote_agent_turn/tests.rs`

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib remote_agent`
- `cargo +1.95.0 check -p openbitfun-desktop`

- [ ] 编码任务的基线、同步与失败（非 Git 工作区、基线分叉）都有测试

## Task 4: 界面展示与两台设备实测

**Outcome:** 智能体列表与子会话正确显示远程条目、设备、等待审批与失败原因，并在两台同账号设备上实测通过。

**Files:**
- Modify: `src/web-ui/src/app/scenes/agents/AgentsScene.tsx`
- Modify: `src/web-ui/src/app/scenes/agents/agentsStore.ts`
- Modify: `src/web-ui/src/flow_chat/` 中 Task 工具卡片相关组件
- Modify: `src/web-ui/src/locales/*/` 相关文案
- Modify: `docs/superplan/plans/features/F002/02-same-account-delegation.md`
- Test: 对应组件的 Vitest 测试

**Verification:**
- `pnpm --dir src/web-ui run test:run <changed test files>`
- `pnpm run type-check:web`
- `pnpm run i18n:audit`
- 手动：两台同账号设备分别实测纯消息、编码、取消、审批、发起方重启补收、目标离线；结果记录在本计划中

- [ ] 界面测试通过，两台设备的实测结果已记录

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/01-target-open-catalog.md`
- `docs/superplan/plans/features/F001/02-task-delegation.md`
- `docs/superplan/plans/features/F001/03-agent-list.md`
- `docs/architecture/detached-task-dispatch.md`
- `docs/architecture/peer-device-mode.md`
