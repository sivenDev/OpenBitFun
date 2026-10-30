---
id: "F002-03"
title: "同账号委派给远程设备上的 ACP 外部智能体"
type: "feature"
status: "approved"
summary: "目标设备可以开放任意已启用的 acp: 外部智能体（如 Claude Code、Codex）；委派任务的 Dispatch 工作进程用目标设备自己的登录或凭据运行它，权限请求进入目标端权限邮箱，目标端声明 external_agent_executor_v1。"
source: "docs/superplan/human/features.md"
created: "2026-10-30"
order: 3
depends_on: ["F001-04", "F002-02"]
parent: "F002"
---

# 同账号委派给远程设备上的 ACP 外部智能体 Plan

**Goal:** 助理能把任务交给同账号另一台设备上启用的 ACP 外部智能体（首个验证目标为 Claude Code），任务在那台设备上用它自己的登录或凭据执行，结果回到发起会话。

**Scope:**
- **开放外部智能体**：开放清单可以选择目标端任意已启用的 `acp:<client_id>`，执行路径不按具体智能体分支。开放前按智能体给出提示：
  - 通用：委派会消耗目标主人在该智能体服务商处的额度，请确认其服务条款；
  - Claude Code：订阅登录只用于个人日常使用，建议使用 API Key。

  外部智能体在目标端未启用或不可运行时不能开放，已开放的条目在目录中显示不可用及原因。
- **能力声明**：目标端只在 ACP 执行器可用时声明 `external_agent_executor_v1`；发起方只在目标声明了该能力时显示远程外部智能体。
- **工作进程执行**：只有带委派、且开放项为 `acp:` 的 Dispatch 任务，工作进程才注册真实的 ACP 执行器；其他 Dispatch 任务保持 F001-04 的明确不支持。
- **权限**：外部智能体的权限请求按 F001-04 的路由与有效策略判定。需要询问的请求进入 Dispatch 的 `remote` 审批邮箱，`status` 中可见，`answer` 回答后继续；工作进程无人应答时不会挂起。
- **凭据**：凭据只在目标设备使用，任何请求、事件或结果中都不携带外部智能体的登录信息或密钥。
- **结果**：与 F002-02 相同；编码任务的改动以分支形式带回。

**Non-Goals:**
- 不做跨账号授权外部智能体（F002-06）。
- 不在普通（非委派）Dispatch 任务中开放外部智能体。
- 不新增 Claude Agent SDK 等非 ACP 执行器；不说 ACP 的智能体不在范围内。

**Architecture:**
- 工作进程按任务决定执行器：委派到 `acp:` 时注册 `AcpExternalAgentExecutor`，否则沿用 `UnsupportedExternalAgentExecutor`。这个决定在初始化前一次确定，不在运行中切换。
- 权限请求复用内核的权限管理器。Dispatch 工作进程把需要询问的请求持久化为现有的安全展示 DTO，来源类型为 Extension，展示时标明来自外部智能体。
- 跨版本：没有 `external_agent_executor_v1` 的目标不提供远程外部智能体；目录中的 `acp:` 条目只在该能力存在时下发。
- 实施时补充的设计（Task 1）：原先外部智能体只能作为子会话运行（`AgentSpawn`/`Task`），顶层会话把 `acp:` 当作未知模式拒绝；委派任务的会话没有父会话。因此内核新增受控的“顶层外部智能体轮次”（`coordination/primary_external_turn.rs`）：
  - 只有调用过 `allow_primary_external_agent_turns()` 的进程启用，目前只有处理委派到 `acp:` 的 Dispatch 工作进程会调用，其他宿主行为不变；
  - 轮次沿用协调器的生命周期（轮次记录、Processing 状态、结算、`cancel_dialog_turn`、终止事件），跳过原生提示词、工具与模型准备；
  - 权限路由与有效策略和子会话相同，策略从轮次元数据中的权限模式读取。
  
  发起方在过滤目录时，对没有该能力的目标丢弃 `acp:` 条目（`remote_agents.rs`）。

**Baseline:**
- `src/apps/cli/src/dispatch/worker.rs` 在初始化前调用 `disable_external_agents_for_process`，工作进程中的外部智能体一律返回 `UnsupportedContext`（F001-04）。
- 外部智能体的权限请求经 `ExternalAgentPermissionRoute` 与有效策略判定（`external_agent_turn.rs`），已有自动批准与询问两条路径。
- Dispatch `remote` 审批策略会持久化待审批请求，并通过 `status` 与 `answer` 处理。

**Exit Criteria:**
- 目标设备开放外部智能体前出现与该智能体对应的额度与条款提示，确认后才开放。
- 从另一台同账号设备委派给目标设备上的 Claude Code，以及另一个 ACP 智能体（如 Codex），任务都在目标设备上运行，回答回到发起会话。
- 外部智能体的写文件或执行命令请求出现在发起方的「等待目标设备审批」中，回答后继续；拒绝后外部智能体收到拒绝。
- 目标设备未启用该外部智能体、或目标版本没有声明能力时，发起方看不到该条目；强行委派时明确失败。
- 普通 Dispatch 任务中的外部智能体仍明确报不支持。

## Task 1: 工作进程按委派启用外部智能体并声明能力

**Outcome:** 委派到 `acp:` 开放项的工作进程能运行对应的外部智能体，目标端在条件满足时声明 `external_agent_executor_v1`。

**Files:**
- Modify: `src/apps/cli/src/dispatch/worker.rs`
- Modify: `src/apps/cli/src/agent/agentic_system.rs`
- Modify: `src/apps/cli/src/dispatch/mod.rs`
- Modify: `src/crates/services/services-core/src/dispatch_contract.rs`
- Modify: `src/web-ui/src/features/dispatch/dispatch.contract.test.ts`
- Test: `src/apps/cli/src/dispatch/mod.rs`

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `cargo +1.95.0 test --locked -p openbitfun-cli --bin openbitfun agentic_system`
- `pnpm --dir src/web-ui run test:run src/features/dispatch/dispatch.contract.test.ts`

- [x] 只有委派到 `acp:` 的任务启用执行器，能力只在执行器可用时声明
  - 证据：`dispatch::` 104 项、`agentic_system`、`dispatch.contract.test.ts` 11 项通过；核心 `external_agent` 35 项通过，其中包括顶层轮次未启用时拒绝、启用后完成并结算、取消传到执行器；`service::dispatch` 65 项通过，其中包括没有能力的目标不列出 `acp:` 条目；CLI 合约测试 `targets_advertise_delegation_to_external_agents` 用真实二进制确认 probe 声明了该能力。

## Task 2: 开放外部智能体与提示

**Outcome:** 开放清单可以选择任意已启用的外部智能体，开放前按智能体给出额度与条款提示，目录显示其可用状态。

**Files:**
- Modify: `src/crates/assembly/core/src/service/delegation.rs`
- Modify: `src/web-ui/src/infrastructure/config/components/AgentDelegationConfig.tsx`
- Modify: `src/apps/cli/src/delegation_cli.rs`
- Modify: `src/web-ui/src/locales/*/` 相关文案
- Test: `src/crates/assembly/core/src/service/delegation.rs`

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib service::delegation`
- `pnpm --dir src/web-ui run test:run <changed test files>`
- `pnpm run i18n:audit`

- [ ] 外部智能体的开放需要确认，不可用时不能开放

## Task 3: 工作进程中的权限请求进入审批邮箱，并实测

**Outcome:** 外部智能体在工作进程中的权限请求能经 Dispatch 审批邮箱回答，并在两台同账号设备上实测。

**Files:**
- Modify: `src/apps/cli/src/dispatch/permissions.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/external_agent_turn.rs`
- Modify: `docs/superplan/plans/features/F002/03-remote-external-agents.md`
- Test: `src/apps/cli/src/dispatch/permissions.rs`

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib external_agent`
- 手动：两台同账号设备分别用 Claude Code 和另一个 ACP 智能体实测纯消息、编码、审批通过与拒绝、取消；确认目标端使用自己的登录或凭据，结果记录在本计划中

- [ ] 审批往返有测试，两台设备的实测结果已记录

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/rfcs/F001.md`
- `docs/superplan/plans/features/F001/04-remote-scenarios.md`
- `docs/superplan/plans/features/F002/02-same-account-delegation.md`
- `docs/architecture/detached-task-dispatch.md`
