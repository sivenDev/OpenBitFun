---
id: "F002-07"
title: "AI协作：只开放助理与连接的信任级别"
type: "feature"
status: "approved"
summary: "把 F002 收敛到 AI协作基准：开放对象只有助理，删除模式与外部智能体开放项及编码任务；每个外部任务是该助理名下的独立会话，助理以完整能力处理；跨账号连接分普通与完全信任两级；外部任务不改写助理。"
source: "docs/superplan/human/features.md"
created: "2026-10-31"
order: 7
depends_on: ["F002-06"]
parent: "F002"
---

# AI协作：只开放助理与连接的信任级别 Plan

**需求基准：** RFC `docs/superplan/rfcs/F002.md` 的「AI协作需求（基准）」一节。实现与评审不偏离该节，除非主人修改需求；不服务于基准的功能不保留。

**Goal:** 主人可以把自己的助理交给自己的其他设备和信任的账号使用，助理完整地解决对方的问题，主人只用一个选择决定每个连接能碰到自己的哪些东西。

**Scope:**
- **只开放助理**：开放项只能是本机的一个助理；开放时对助理可能调用的外部智能体确认一次额度与条款；展示名默认取助理名称；目录不暴露人设、记忆或路径。
- **删除不服务于基准的功能**：
  - 按智能体模式开放、按 ACP 外部智能体开放的开放项（含界面、CLI、桌面命令与测试）；
  - 委派编码任务：发起方 `Task` 的 `task_kind`、基线准备与分支同步（`sync_coding_result`）、目标端委派路径上的编码任务与工作区准备；
  - 工作进程以外部智能体作为任务主体运行的路径（`primary_external_turn.rs`）。外部智能体保留为助理任务中的子智能体。
  - Detached Dispatch 自身的通用编码任务（非委派）不在删除范围内。
- **升级兼容**：旧的模式与外部智能体开放记录保留、照常读取，标为「已不支持」，不进目录、不接受委派，主人可以删除；委派到它们、或旧版发起方提交编码任务，都被明确拒绝并说明原因。
- **外部任务以助理身份运行**：纯消息任务；独立工作目录；提示中总是包含 `SOUL.md` 与 `IDENTITY.md`；助理平时的工具、子智能体与外部智能体都可用，受信任级别约束。
- **连接的信任级别**：
  - 跨账号连接分「普通」（默认）与「完全信任」，由目标主人按连接选择；改为完全信任前显示风险并确认。同账号设备按完全信任处理。
  - 普通级别：任务及其子智能体不能使用 `SessionHistory`、`SessionMessage`、`SessionControl`、`ListWorkspaces`、`ComputerUse`、`ControlHub`、`Cron`、`create_goal`、`update_goal`、`PageDeploy`、`PagePublish`、`PublishAppearance`、`OpenBitFunControl`，拒绝时给出原因；不能选择被禁止的子智能体类型；提示中不包含记忆摘要与 `USER.md`；文件工具读取 `USER.md` 或记忆目录总是需要主人审批。界面如实说明普通级别对记忆的保护依靠审批。
  - 完全信任：以上都可以使用，提示中包含记忆摘要与 `USER.md`。
  - 修改级别对之后开始的回合生效；审批人、额度、审计、撤销不变。
- **不改写助理**（不分信任级别）：外部任务不参与记忆提取与整理；整个助理工作区只读（含人设、`BOOTSTRAP.md` 与工作区指令文件）。
- **主人视角**：外部任务显示为该助理名下的只读会话，标明来源账号与设备，可以查看过程、处理审批、看到结束状态；审批经 Dispatch 权限邮箱，手机端、IM 与 `dispatch answer` 同样可以处理；Peer Device Mode 下明确显示不支持。
- **审计**：跨账号审计增加任务开始时的信任级别与任务调用的外部智能体。

**Non-Goals:**
- 不做按助理隔离的记忆；不对命令执行与外部智能体做文件级沙箱。
- 不做多跳委派。
- 不删除 Detached Dispatch 的通用能力（编码任务、工作区准备、SSH 目标），只删除委派路径上的使用。
- 不删除 F001 本机委派外部智能体的能力。

**Architecture:**
- **助理开放项**：以旧版工作进程不认识的智能体类型表示（`assistant:<workspaceId>`），旧进程按未知模式明确拒绝；新增协商能力 `assistant_delegation_v1`，发起方只对声明了它的目标展示助理。
- **删除路径**：`OpenAgent` 校验只接受助理类型；读取旧记录时保留并标记为不支持（不改写磁盘）。委派协议的任务类型仍可反序列化旧值，目标端对 `coding` 返回明确拒绝。
- **信任判定**：工作进程在回合开始时读取连接的信任级别写入 `dispatchDelegation` 元数据；内核据此并入 `ToolRuntimeRestrictions`，子智能体上下文传递同一元数据。
- **不改写助理**：任务工作区是独立目录；助理工作区与记忆目录作为只读路径提供；带委派标记的会话不进入记忆提取。
- **主人视角**：桌面端经本机 Dispatch 动词（`list`、`status`、`answer`）读取委派到本机助理的任务，作为投影显示在助理名下，不写入助理的会话存储。

**Baseline:**
- 开放项的 `agent_type` 可以是任何 `Mode` 类别的智能体或 `acp:<client>`（`service/delegation.rs`）；外部智能体开放有额度与条款确认（`ExternalAgentUsageNotice`）。
- `DelegationTaskKind` 有 `message` 与 `coding`，贯穿发起方 `Task` 工具（`task/schema.rs`、`input.rs`、`execution.rs`）、`remote_agent_turn.rs`、`runtime-ports/remote_agent.rs`（`sync_coding_result`）、CLI `dispatch/{protocol,connection,delegation,worker,mod,audit}.rs` 与 Web UI。
- 工作进程通过 `allow_primary_external_agent_turns` 让 `acp:` 开放项作为任务主体运行。
- `dispatchDelegation` 目前只用于禁止远程委派；委派任务使用被开放模式的全部默认工具；Claw 的默认工具与记忆摘要会进入其他账号的任务。
- 记忆根目录对所有会话共享；记忆提取准入在 `memories/startup.rs::memory_startup_is_eligible`，不识别委派会话。

**Exit Criteria:**
- RFC 批准条件 1–11 在两账号模拟中逐项验证并记录。
- 代码中不再有开放模式或外部智能体、委派编码任务、外部智能体作为任务主体的路径；旧记录与旧版发起方得到明确拒绝。

## Task 1: 助理开放项，外部任务以助理身份运行且不改写助理

**Outcome:** 主人可以开放一个助理（确认外部智能体的额度与条款）；工作进程在独立工作目录中以助理的人设和平时的工具运行外部任务；助理工作区只读；外部任务不进入记忆提取。

**Files:**
- Modify: `src/crates/assembly/core/src/service/delegation.rs`
- Modify: `src/apps/cli/src/delegation_cli.rs`
- Modify: `src/apps/cli/src/dispatch/{worker,connection,delegation,mod}.rs`
- Modify: `src/crates/services/services-core/src/dispatch_contract.rs`（`assistant_delegation_v1`）
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordinator.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/prompt_builder/prompt_builder_impl.rs`
- Modify: `src/crates/assembly/core/src/service/bootstrap/bootstrap_impl.rs`
- Modify: `src/crates/assembly/core/src/agentic/memories/startup.rs`
- Modify: `src/apps/desktop/src/api/delegation_api.rs`
- Test: 上述模块测试；`src/apps/cli/tests/cli_command_contracts/delegation_simulation.rs`

**Change Map:**
- `service/delegation.rs`: 助理开放项（`assistant:<workspaceId>`）与校验（助理存在）；外部智能体额度与条款确认改为开放助理时确认一次；旧记录反序列化与往返测试。
- `worker.rs`: 助理任务以任务独立工作目录为工作区，加载助理人设，回合元数据带上助理；助理工作区与记忆目录作为只读路径。
- `bootstrap_impl.rs`、`prompt_builder_impl.rs`: 从助理工作区构建人设（`SOUL.md`、`IDENTITY.md`）。
- `coordinator.rs`: 路径策略只允许写任务工作目录。
- `memories/startup.rs`: 带委派标记的会话不参与记忆提取。
- `dispatch_contract.rs`、`connection.rs`、`delegation.rs`: 声明并检查 `assistant_delegation_v1`。
- `delegation_cli.rs`、`delegation_api.rs`: `delegation open --assistant <id>`；列出可开放的助理。

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib delegation`
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib memories`
- `cargo test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `cargo test --locked -p openbitfun-cli --test cli_command_contracts delegation`（开放带人设的助理：回答体现人设；写入助理工作区被拒；记忆提取不收录外部任务）
- `cargo check -p openbitfun-desktop --lib`

- [x] 助理可以被开放并以助理身份处理外部任务，助理工作区只读、不进入记忆提取，均有测试

**Evidence (2026-10-31):**
- 开放项 `assistant:<目录>`（`workspace` 或 `workspace-<id>`），按目录识别，无需工作区目录服务，CLI 与桌面都能列出；开放需确认外部智能体的额度与条款；只接受纯消息任务。
- 目标端准入为每个任务建独立目录（`delegation/workspaces/<开放项>/<发起方>/<任务>`），由本机写入 `__assistantWorkspace`，拒绝发起方自带；助理被删除后拒绝新任务。
- 工作进程以 `Claw` 模式运行，回合元数据带助理、任务目录与信任（同账号完全信任、连接普通，任务 3 改为读取连接设置）；外部智能体对助理任务可用。
- 内核：人设只读取 `SOUL.md`、`IDENTITY.md`（完全信任另加 `USER.md`），不创建、不写入；写、改、删只允许任务目录，子智能体继承；普通信任不注入记忆；委派目录不参与记忆提取与扫描。
- 测试：`cargo test -p openbitfun-core ... --lib -- delegation memories::startup bootstrap` 57 通过，`-- child_context tools::implementations::task remote_agent` 125 通过；`cargo test -p openbitfun-cli --bin openbitfun dispatch::` 118 通过；`cargo check -p openbitfun-desktop --lib` 通过。
- 偏差：未新增 `assistant_delegation_v1`。旧目标的开放清单里不会有助理，旧发起方按目录里的类型提交即可被新目标接受，能力声明没有消费方。端到端模拟在任务 2 把模拟改为助理后一并验证。

## Task 2: 删除模式与外部智能体开放项、委派编码任务

**Outcome:** 开放项只剩助理；委派路径上不再有编码任务与外部智能体作为任务主体；旧记录保留并标为不支持，委派到它们与旧版发起方的编码任务被明确拒绝。

**Files:**
- Modify: `src/crates/assembly/core/src/service/delegation.rs`、`delegation_audit.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/{schema,input,execution,agent_control,tests}.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/{remote_agent_turn.rs,remote_agent_turn/tests.rs,coordinator.rs}`
- Delete: `src/crates/assembly/core/src/agentic/coordination/primary_external_turn.rs`（及其测试中只针对它的部分）
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/{remote_agents,types}.rs`、`src/crates/assembly/core/src/service/dispatch/remote_agents.rs`
- Modify: `src/crates/contracts/runtime-ports/src/remote_agent.rs`、`tests/remote_agent_port_contracts.rs`
- Modify: `src/apps/cli/src/dispatch/{protocol,connection,connection_guard,delegation,worker,mod,audit}.rs`、`src/apps/cli/src/delegation_cli.rs`、`src/apps/cli/src/agent/agentic_system.rs`
- Modify: `src/apps/desktop/src/api/{delegation_api.rs,remote_agent_delegation/*}`
- Modify: `src/apps/cli/tests/cli_command_contracts/delegation_simulation.rs`、`src/apps/desktop/src/api/remote_agent_delegation/simulation_tests.rs`

**Change Map:**
- 开放项校验只接受助理；旧的模式与 `acp:` 记录读取后标为不支持，不进目录，委派返回「已不再开放」。
- 删除发起方 `task_kind` 参数与编码路径（基线准备、`sync_coding_result`、结果中的改动文件列表）；`Task` 工具说明改为只描述远程助理。
- 目标端：委派协议仍能解析旧的 `coding` 值，返回「目标只接受消息任务」；删除委派编码任务的工作区准备与交付；删除 `allow_primary_external_agent_turns` 与 `primary_external_turn.rs`。
- 审计：保留旧记录中的任务类型字段（可选），新记录不再写入。
- 删除只覆盖已删除功能的测试，改写其余测试以使用助理开放项。

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib delegation`
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib remote_agent`
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib tools::implementations::task`
- `cargo test --locked -p openbitfun-runtime-ports`
- `cargo test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `cargo test --locked -p openbitfun-cli --test cli_command_contracts delegation`
- `cargo check -p openbitfun-desktop --lib`；`pnpm run check:core-boundaries`

- [ ] 已删除的功能没有残留入口，旧记录与旧版发起方得到明确拒绝，均有测试

## Task 3: 连接的信任级别

**Outcome:** 跨账号外部任务按连接的信任级别受限：普通级别在工具层禁止触碰主人的桌面、其他会话、定时与发布，不提供记忆与 `USER.md`，文件工具读取它们需要审批，子智能体继承限制；完全信任与同账号一致；审计记录信任级别与调用的外部智能体。

**Files:**
- Modify: `src/crates/assembly/core/src/service/delegation.rs`、`delegation_audit.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordinator.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/execution.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/prompt_builder/prompt_builder_impl.rs`
- Modify: `src/apps/cli/src/dispatch/{worker,audit}.rs`、`src/apps/cli/src/delegation_cli.rs`
- Modify: `src/apps/desktop/src/api/delegation_api.rs`、`src/crates/contracts/product-domains/src/remote_surface/table.rs`
- Test: `src/crates/assembly/core/src/agentic/tools/implementations/task/tests.rs`、`delegation_simulation.rs`

**Change Map:**
- `service/delegation.rs`: 按连接保存信任级别（可选字段，旧数据为普通）；普通级别禁止的工具名单与拒绝原因（单一来源）；从委派元数据判定是否受限。
- `worker.rs`: 回合开始时把信任级别写入委派元数据。
- `coordinator.rs`: 普通级别的限制并入 `ToolRuntimeRestrictions`；文件工具读取 `USER.md` 与记忆目录固定需要审批。
- `prompt_builder_impl.rs`: 普通级别不注入记忆摘要与 `USER.md`；完全信任与同账号注入。
- `task/execution.rs`: 子智能体上下文传递委派元数据；普通级别下拒绝被禁止的子智能体类型。
- 审计：信任级别与调用的外部智能体（可选字段）。
- `delegation_cli.rs`、`delegation_api.rs`、`table.rs`: `delegation trust-connection <id> --level normal|full`（完全信任需确认）；桌面读写命令登记 Product Operation Registry。

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib delegation`
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib tools::implementations::task`
- `cargo test --locked -p openbitfun-cli --test cli_command_contracts delegation`（普通连接的被禁工具与子智能体被拒绝、提示中没有记忆；改为完全信任后可用；同账号不受影响）
- `pnpm run capabilities:check`；`cargo test --locked -p openbitfun-product-domains remote_surface`

- [ ] 普通级别的限制在工具层生效且子智能体继承，完全信任与同账号一致，审计记录信任级别，均有测试

## Task 4: AI协作页与助理名下的外部任务

**Outcome:** AI协作页只开放助理（含外部智能体额度确认）；旧的模式与外部智能体开放记录显示为已不支持并可删除；连接列表可以修改信任级别（完全信任需确认，说明记忆是全局的、普通级别依靠审批）；外部任务显示在该助理名下，标明来源，主人可以查看与审批；发起方界面不再出现编码任务；Peer Device Mode 下显示不支持。

**Files:**
- Modify: `src/web-ui/src/infrastructure/config/components/{AgentDelegationConfig,ConnectionsConfig,DelegationAuditView}.tsx` 及测试
- Modify: `src/web-ui/src/infrastructure/api/service-api/{DelegationAPI,SubagentAPI}.ts`
- Modify: `src/web-ui/src/app/scenes/agents/AgentsScene.tsx` 及测试
- Modify: `src/web-ui/src/app/components/NavPanel/workspaceSessionView.ts` 及测试
- Modify: `src/apps/desktop/src/api/dispatch_api.rs`（如现有命令不足）
- Modify: `src/web-ui/src/locales/{en-US,zh-CN,zh-TW}/settings/runtime.json`、`scenes/agents.json`

**Change Map:**
- `AgentDelegationConfig.tsx`: 只列可开放的助理；删除模式与外部智能体的选择；旧记录标为不支持并可删除。
- `ConnectionsConfig.tsx`: 每个连接的信任级别与确认。
- `AgentsScene.tsx`、`SubagentAPI.ts`、`DelegationAuditView.tsx`: 删除编码任务相关的展示与文案。
- `workspaceSessionView.ts`: 把本机外部任务投影为该助理名下的只读会话条目，带来源与状态。
- `dispatch_api.rs`: 本机外部任务的列表、状态与审批回答；新命令登记 Product Operation Registry，Peer Device Mode 下为 `ControllerLocal` 或拒绝。

**Verification:**
- `pnpm --dir src/web-ui run test:run src/infrastructure/config/components src/app/scenes/agents src/app/components/NavPanel/workspaceSessionView.test.ts`
- `pnpm run type-check:web`、`pnpm run lint:web`、`pnpm run i18n:audit`
- 新增桌面命令时：`pnpm run capabilities:check`

- [ ] AI协作页、信任级别与助理名下的外部任务在界面上可用，已删除功能无残留，均有测试

## Task 5: 端到端验证与记录

**Outcome:** 在两账号模拟中逐项验证 RFC 批准条件 1–11，结果记录在本计划中。

**Files:**
- Modify: `src/apps/desktop/src/api/remote_agent_delegation/simulation_tests.rs`
- Modify: `src/apps/cli/tests/cli_command_contracts/delegation_simulation.rs`
- Modify: `scripts/dev/sim-devices.mjs`（模拟设备带一个助理）
- Modify: `docs/superplan/plans/features/F002/07-open-assistants.md`

**Verification:**
- `RUSTUP_TOOLCHAIN=1.95.0 node scripts/dev/sim-devices.mjs --check`
- 在模拟客户端中查看 AI协作页与助理名下的外部任务、处理一次审批（截图记录）
- 模拟环境中没有真实的 ACP 智能体：助理调用外部智能体由内核测试与模拟 ACP 客户端覆盖，结果中注明

- [ ] RFC 批准条件 1–11 逐项实测并记录

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/06-cross-account-delegation.md`
