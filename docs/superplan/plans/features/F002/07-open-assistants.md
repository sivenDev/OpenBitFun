---
id: "F002-07"
title: "跨账号工具上限与开放助理"
type: "feature"
status: "draft"
summary: "跨账号委派任务及其子智能体在工具层受上限约束；目标主人可以把助理开放出去，每个任务是该助理名下的独立会话，记忆读取范围按授权选择，委派会话不改动助理。"
source: "docs/superplan/human/features.md"
created: "2026-10-31"
order: 7
depends_on: ["F002-06"]
parent: "F002"
---

# 跨账号工具上限与开放助理 Plan

**Goal:** 主人可以放心地把自己的助理交给别人使用：别人的任务带着助理的人设解决问题，却碰不到主人的桌面、其他会话和未授权的记忆，也改变不了这个助理。

**Scope:**
- **跨账号工具上限（第一部分，修正现状）**：
  - 来自其他账号的委派任务，无论开放的是哪个智能体模式（包括 Claw 和自定义智能体），都不能使用会话与工作区类工具（`SessionHistory`、`SessionMessage`、`SessionControl`、`ListWorkspaces`），不能控制桌面、浏览器或终端会话（`ComputerUse`、`ControlHub`），不能创建定时任务或持久目标（`Cron`、`create_goal`、`update_goal`），不能发布（`PageDeploy`、`PagePublish`、`PublishAppearance`），不能操作 OpenBitFun 自身（`OpenBitFunControl`），也不注入记忆摘要（D12 授予记忆的助理除外）。
  - 被拒绝时工具返回明确原因。
  - 子智能体继承上述限制与委派标记；不能选择被禁止的智能体类型（如 `ComputerUse`）；跨账号任务中不能启动 `acp:` 外部智能体；子智能体随任务的取消、超时与连接撤销一起停止。
  - 同账号委派任务不受工具上限影响。
- **开放助理**：
  - 开放清单可以选择本机的一个助理，与智能体模式分组展示；模式条目注明「只借用本机的模型、工具和凭据，不带身份和记忆」。
  - 助理开放项只接受纯消息任务。展示名与说明默认取助理名称，目录不暴露人设内容、记忆或路径。
  - 每个授权带记忆读取范围：「仅人设」或「人设与记忆」。跨账号默认仅人设，同账号默认人设与记忆；跨账号改为人设与记忆需要主人显式确认，并提示读取的是全局记忆（来自所有会话）。
  - **仅人设**：任务在每个任务独立的空工作区中运行，目标端把 `SOUL.md` 与 `IDENTITY.md` 放进提示；不提供命令执行工具，不注入记忆摘要，看不到助理工作区。
  - **人设与记忆**：任务在助理工作区中运行，可以读取 `USER.md` 与记忆，命令执行按开放项的审批策略处理。
  - **不改动助理**（同账号与跨账号都生效）：委派会话不参与记忆提取与整理；人设文件只读；不能创建定时任务或持久目标。
- **主人视角**：
  - 委派来的任务作为该助理名下的会话展示，标明来源账号与设备，可以查看过程、处理审批、看到结束状态。
  - 会话由 Dispatch 任务单一写入，桌面端只读展示；审批经 Dispatch 权限邮箱，手机端、IM 与 `dispatch answer` 同样可以处理。
  - 同一任务的追问延续同一会话。

**Non-Goals:**
- 不做按助理隔离的记忆（记忆仍是全局的一份）。
- 助理开放项不接受编码任务。
- 不改变同账号委派任务的工具集合。
- 不做多跳委派；不改变发起方的协议与界面（发起方看到的仍是普通远程智能体）。
- 不提供给主人放宽跨账号工具上限的开关。

**Architecture:**
- **上限的判定与执行**：工作进程写入的 `dispatchDelegation` 回合元数据已含 `sourceConnectionId`。内核据此判定跨账号任务，沿用现有 `ToolRuntimeRestrictions`（与连接级临时会话禁用工具的做法相同）在工具层拒绝；记忆摘要在提示构建处按同一判定关闭。父任务的运行时限制本来就约束子智能体，计划只需补上委派标记中的账号来源与记忆范围在子智能体上下文中的传递，以及子智能体类型与 `acp:` 的准入检查。
- **助理开放项的标识**：开放项以旧版工作进程不认识的智能体类型表示（如 `assistant:<workspaceId>`），旧进程按未知模式明确拒绝，不会当作普通 Claw 执行。记录与授权的新字段都是可选的，旧记录照旧读取。
- **记忆范围的执行**：由工作进程在提交时按授权决定工作区与回合元数据；内核按元数据选择人设来源、工具上限、路径策略（人设文件不可写）与记忆摘要。提示词只描述身份，不承担隔离。
- **主人视角**：桌面端经本机的 Dispatch 动词（`list`、`status`、`answer`）读取委派到本机助理的任务，作为投影显示在助理名下，不写入助理的会话存储。

**Baseline:**
- `dispatchDelegation` 目前只用于禁止远程委派（`task/execution.rs` 的 `remote_agent_parent_refusal`），委派任务使用被开放模式的全部默认工具。
- Claw 的默认工具含上述会话、桌面控制、定时与发布类工具，`user_context_policy` 含记忆摘要；任何 `Mode` 类别的智能体都可以被开放（`service/delegation.rs` 只排除 `remote:`）。
- 记忆根目录是产品目录下的 `memories`（`PathManager::memories_root_dir`），对所有会话共享。
- 记忆提取的准入在 `memories/startup.rs::memory_startup_is_eligible`，目前不识别委派会话。
- 目标端执行始终经 `openbitfun dispatch` 工作进程；桌面端只转发（`dispatch_host.rs`）。

**Exit Criteria:**
- 两账号模拟中，把 Claw 按模式开放给连接后，对方任务调用会话历史、定时、电脑操作与发布类工具都被拒绝并给出原因，也拿不到记忆摘要；它启动的子智能体同样被拒绝，启动 `acp:` 子智能体被拒绝；同账号对照组不受影响。
- 主人把一个助理开放给同账号设备与跨账号连接后，两边都能委派纯消息任务：回答体现该助理的人设；每个任务在主人那边显示为该助理名下标明来源的会话；追问延续同一会话；主人能在那里批准或拒绝审批。
- 仅人设授权下，任务被直接要求时也无法经文件读取、命令执行或会话历史拿到 `USER.md`、记忆或助理工作区内容；人设与记忆授权下可以读到。
- 委派会话结束并经过记忆整理后，记忆中不出现来自它的内容；要求修改人设文件或创建定时任务被拒绝。
- 旧版工作进程收到助理开放项时明确拒绝；旧版开放记录与授权照常读取。
- RFC 批准条件 1–8 的回归检查仍然通过。

## Task 1: 跨账号委派任务的工具上限与子智能体限制

**Outcome:** 来自其他账号的委派任务及其子智能体在工具层被限制在 RFC D10 的上限内，拒绝时有明确原因；同账号任务的工具集合不变。这部分修正已发布 F002 中按模式开放 Claw 的暴露面。

**Files:**
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordinator.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/execution.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/prompt_builder/prompt_builder_impl.rs`
- Modify: `src/crates/assembly/core/src/service/delegation.rs`
- Modify: `src/apps/cli/src/dispatch/worker.rs`
- Test: `src/crates/assembly/core/src/agentic/tools/implementations/task/tests.rs`
- Test: `src/apps/cli/tests/cli_command_contracts/delegation_simulation.rs`

**Change Map:**
- `service/delegation.rs`: 跨账号工具上限的工具名单与拒绝原因（单一来源），以及从委派元数据判定跨账号的函数。
- `coordinator.rs`: 回合开始时按委派元数据把上限并入 `ToolRuntimeRestrictions`。
- `prompt_builder_impl.rs`: 跨账号委派回合不注入记忆摘要。
- `task/execution.rs`: 子智能体上下文传递完整的委派元数据；拒绝被禁止的子智能体类型与跨账号任务中的 `acp:` 子智能体。
- `worker.rs`: 委派元数据保持现有字段，供内核判定（如有缺失则补齐）。

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib delegation`
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib tools::implementations::task`
- `cargo test --locked -p openbitfun-cli --test cli_command_contracts delegation`（两账号模拟：按模式开放 Claw，跨账号任务的被禁工具与子智能体被拒绝，同账号对照不受影响）

- [ ] 跨账号委派任务及其子智能体无法使用被禁止的工具、拿不到记忆摘要、不能启动 `acp:` 子智能体，同账号任务不变，均有测试

## Task 2: 助理开放项与按授权的记忆范围

**Outcome:** 主人可以把一个助理开放出去，并为每个授权选择记忆范围；记录兼容旧版本，旧版工作进程明确拒绝助理开放项。

**Files:**
- Modify: `src/crates/assembly/core/src/service/delegation.rs`
- Modify: `src/apps/cli/src/delegation_cli.rs`
- Modify: `src/apps/cli/src/dispatch/connection.rs`
- Modify: `src/apps/cli/src/dispatch/mod.rs`
- Modify: `src/apps/desktop/src/api/delegation_api.rs`
- Modify: `src/crates/contracts/product-domains/src/remote_surface/table.rs`（仅在新增桌面命令时）
- Test: `src/crates/assembly/core/src/service/delegation.rs`（模块测试）
- Test: `src/apps/cli/src/dispatch/connection.rs`（模块测试）

**Change Map:**
- `service/delegation.rs`: 开放项新增助理类型（`assistant:<workspaceId>`）及校验（助理存在、只允许纯消息）；授权新增可选的记忆范围，默认值按同账号与跨账号区分；跨账号授予记忆需要确认标记；旧记录的读取与往返测试。
- `delegation_cli.rs`: `delegation open` 支持选择助理；授权命令支持记忆范围与确认。
- `dispatch/connection.rs`、`dispatch/mod.rs`: 提交时拒绝对助理开放项的编码任务。
- `delegation_api.rs`: 列出可开放的助理；读写授权的记忆范围。

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib delegation`（含旧记录反序列化与往返）
- `cargo test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `cargo check -p openbitfun-desktop --lib`；新增桌面命令时另跑 `pnpm run capabilities:check` 与 `cargo test --locked -p openbitfun-product-domains remote_surface`

- [ ] 助理可以被开放、按授权选择记忆范围，旧记录兼容，编码任务与旧版进程明确拒绝，均有测试

## Task 3: 工作进程按记忆范围运行助理任务，委派会话不改动助理

**Outcome:** 助理任务按授权的记忆范围运行：仅人设时在空工作区、只带 `SOUL.md` 与 `IDENTITY.md`、无命令执行与记忆；人设与记忆时在助理工作区运行。任何委派会话都不参与记忆提取，人设文件只读，不能创建定时任务或持久目标。

**Files:**
- Modify: `src/apps/cli/src/dispatch/worker.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordinator.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/prompt_builder/prompt_builder_impl.rs`
- Modify: `src/crates/assembly/core/src/service/bootstrap/bootstrap_impl.rs`
- Modify: `src/crates/assembly/core/src/agentic/memories/startup.rs`
- Test: `src/crates/assembly/core/src/agentic/memories/startup.rs`（模块测试）
- Test: `src/apps/cli/tests/cli_command_contracts/delegation_simulation.rs`

**Change Map:**
- `worker.rs`: 助理任务按记忆范围选择工作区（任务独立空目录或助理工作区）并写入回合元数据（助理、记忆范围）。
- `bootstrap_impl.rs`、`prompt_builder_impl.rs`: 仅人设时从助理工作区只取 `SOUL.md` 与 `IDENTITY.md` 构建人设，不注入记忆摘要。
- `coordinator.rs`: 助理任务的限制：仅人设去掉命令执行工具；人设文件不可写（路径策略）；`Cron` 与持久目标工具被拒绝。
- `memories/startup.rs`: 带委派标记的会话不参与记忆提取。

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib memories`
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib prompt_builder`
- `cargo test --locked -p openbitfun-cli --test cli_command_contracts delegation`（模拟中开放带人设与 `USER.md` 的助理：仅人设任务读不到 `USER.md` 与记忆、执行命令被拒；人设与记忆任务可以读到；修改人设与创建定时任务被拒；记忆提取不收录委派会话）

- [ ] 两种记忆范围按设计隔离，委派会话不改动助理，均有测试

## Task 4: AI协作页与主人在助理名下查看委派会话

**Outcome:** 开放清单把助理与智能体模式分组展示并说明区别；授权面板可以选择记忆范围（跨账号授予记忆需确认并提示全局记忆）；委派到本机助理的任务显示在该助理名下，标明来源，主人可以查看过程与处理审批。

**Files:**
- Modify: `src/web-ui/src/infrastructure/config/components/AgentDelegationConfig.tsx`
- Modify: `src/web-ui/src/infrastructure/api/service-api/DelegationAPI.ts`
- Modify: `src/web-ui/src/app/components/NavPanel/workspaceSessionView.ts`
- Modify: `src/apps/desktop/src/api/dispatch_api.rs`（读取本机委派任务与回答审批，如现有命令不足）
- Modify: `src/web-ui/src/locales/{en-US,zh-CN,zh-TW}/settings/runtime.json`
- Test: `src/web-ui/src/infrastructure/config/components/AgentDelegationConfig.test.tsx`
- Test: `src/web-ui/src/app/components/NavPanel/workspaceSessionView.test.ts`

**Change Map:**
- `AgentDelegationConfig.tsx`: 助理与模式分组、模式说明、记忆范围选择与确认。
- `workspaceSessionView.ts` 及其所属的助理会话列表：投影本机委派任务为该助理名下的只读会话条目，带来源标识与状态。
- `dispatch_api.rs`: 本机委派任务的列表、状态与审批回答，复用 `dispatch_host` 的 CLI 运行器；新命令登记 Product Operation Registry。

**Verification:**
- `pnpm --dir src/web-ui run test:run src/infrastructure/config/components/AgentDelegationConfig.test.tsx src/app/components/NavPanel/workspaceSessionView.test.ts`
- `pnpm run type-check:web`、`pnpm run lint:web`、`pnpm run i18n:audit`
- 新增桌面命令时：`pnpm run capabilities:check`

- [ ] 开放清单、记忆范围授权与助理名下的委派会话在界面上可用，均有测试

## Task 5: 端到端验证与记录

**Outcome:** 在两账号模拟中逐项验证 RFC 批准条件 9–13，回归条件 1–8，结果记录在本计划中。

**Files:**
- Modify: `src/apps/desktop/src/api/remote_agent_delegation/simulation_tests.rs`
- Modify: `src/apps/cli/tests/cli_command_contracts/delegation_simulation.rs`
- Modify: `scripts/dev/sim-devices.mjs`（模拟设备带一个助理时）
- Modify: `docs/superplan/plans/features/F002/07-open-assistants.md`

**Verification:**
- `RUSTUP_TOOLCHAIN=1.95.0 node scripts/dev/sim-devices.mjs --check`
- 在模拟客户端中手动查看助理名下的委派会话与审批（截图记录）

- [ ] RFC 批准条件 9–13 逐项实测并记录，1–8 回归通过

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/06-cross-account-delegation.md`
