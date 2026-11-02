---
id: "F002-07"
title: "AI协作：只开放助理与连接的信任级别"
type: "feature"
status: "superseded"
summary: "把 F002 收敛到 AI协作基准：开放对象只有助理，删除模式与外部智能体开放项及编码任务；每个外部任务是该助理名下的独立会话，助理以完整能力处理；跨账号连接分普通与完全信任两级；外部任务不改写助理。"
source: "docs/superplan/human/features.md"
created: "2026-10-31"
order: 7
depends_on: ["F002-06"]
parent: "F002"
---

# AI协作：只开放助理与连接的信任级别 Plan

> 已被 F002 RFC 第 6 版（局域网共享助理）取代，替代计划为 F002-10 至 F002-13（待 RFC 批准后制定）。本计划交付的成果中，Dispatch 执行与审批、外部任务以助理身份运行且不改写助理、受阻时如实告知与排队、会话保留期限，按 RFC D12 迁移进新流程，其余随第 6 版删除。

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

- [x] 已删除的功能没有残留入口，旧记录与旧版发起方得到明确拒绝，均有测试

**Evidence (2026-10-31, Task 2):**
- 删除：`RemoteAgentTaskKind`、`sync_coding_result` 与编码结果类型；`Task`/`AgentSpawn` 的 `task_kind`（传入即报错）；发起方出站基线存储与编码提交；`primary_external_turn.rs` 与 `allow_primary_external_agent_turns`；`OpenAgent` 的 `message_workspace`、`open`、`open_external_agent`、`ExternalAgentUsageNotice`；CLI `delegation open --agent/--task-kind/--workspace` 与 `grant-connection --accept-usage`；目标端委派路径上的工作区准备（连接上的 `workspace_*` 动词与带 `workspacePath` 的探测被拒绝）。
- 旧数据：模式与 `acp:` 开放记录保留在磁盘，读取为「no longer supported」，不进目录；委派到它们返回「is no longer open on this device」；旧版发起方提交 `coding` 返回「accepts only message tasks」（同账号与连接两条路径都有测试）；旧版的 `taskKind: coding` 委派链接仍可解析。
- 模拟改为助理：每台模拟设备创建默认助理（`SOUL.md`、`IDENTITY.md`、`USER.md`）并用 `delegation open --assistant workspace --accept-usage` 开放。新增断言：模型请求包含人设；A4 的任务先尝试改写 `SOUL.md`，即使批准也不落盘；连接上的编码提交与工作区准备被拒绝。
- 测试：`cargo test -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib -- delegation remote_agent tools::implementations::task external_agent_turn` 169 通过；`cargo test -p openbitfun-runtime-ports --features remote-agent-port` 8 + 6 通过；`cargo test -p openbitfun-cli --bin openbitfun -- dispatch:: delegation tests::dispatch` 117 + 1 通过；`cargo test -p openbitfun-cli --test cli_command_contracts delegation` 4 通过（2 个由脚本启动的忽略）；`cargo check -p openbitfun-desktop --lib --tests` 通过；`pnpm run check:core-boundaries`、`type-check:web`、`i18n:audit` 通过；`AgentsScene.test.tsx` 21 通过。
- 环境：本机 Xcode 更新后 `/usr/bin/git` 的 xcrun 缓存指向已删除的 SDK，`dispatch::workspace` 的 16 个 Git 测试因此失败；把 Homebrew git 放在 PATH 前面后全部通过，与本任务无关。
- 偏差：
  - 审计新记录仍写 `taskKind`（`message`，或被拒绝的旧版 `coding`）：旧版读取审计时要求该字段，去掉会让旧版读不出新记录。
  - 保留 `external_agent_executor_v1` 能力声明，含义改为助理任务可以调用外部智能体。
  - Relay 与连接传输的可路由命令列表不变（已部署的 Relay 与旧版设备仍使用同一列表），由目标端拒绝工作区动词。
  - Web UI 只删除了远程智能体详情中的任务类型；开放表单（`AgentDelegationConfig.tsx`）仍是旧的模式选择，按计划在任务 4 改为只开放助理，在此之前通过它开放模式会被后端拒绝。
  - 桌面端发起方模拟（`sim-devices.mjs --check`）已改写并通过编译，运行留到任务 5 的端到端验证。

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

- [x] 普通级别的限制在工具层生效且子智能体继承，完全信任与同账号一致，审计记录信任级别，均有测试

**Evidence (2026-10-31, Task 3):**
- 存储：信任级别存放在单独的 `delegation/connection-trust.json`（只记录非普通级别）；旧版重写开放清单时不会丢失它。没有记录、未知级别或文件无法读取时按普通处理；无法解析的文件不会被覆盖。
- 单一来源：`NORMAL_TRUST_DENIED_TOOLS`（13 个工具）与 `NORMAL_TRUST_DENIED_SUBAGENTS`（`ComputerUse`、`OpenBitFun`）。普通级别下 `restrict_tools` 把它们加入 `denied_tool_names` 并附拒绝原因；工具清单里看不到它们；子智能体经同一函数继承。`Task` 工具在新建或继续子智能体时拒绝被禁类型。
- 私密文件：普通级别下，`Read` 读取助理的 `USER.md` 或记忆目录（`memories/` 与记忆数据库目录）、或者 `Grep`/`Glob`/`LS` 的搜索范围包含它们时，追加带 `requiresFreshApproval` 的 `read` 审批。记住的授权、Hook 预批准与自动批准都不能跳过它。
- 工作进程在每个回合开始时读取连接的信任级别（同账号设备按完全信任），修改从下一个回合生效。审计条目新增 `trust` 与 `externalAgents`（任务启动的 `acp:` 子智能体），旧条目照常读取。
- 管理入口：CLI `delegation trust-connection <connection> --level normal|full`（完全信任需 `--accept-risk`，并说明记忆对所有助理共享；改回普通时说明命令与外部智能体只靠审批保护），`delegation list` 显示每个连接的信任级别；桌面命令 `delegation_list_connection_trust`、`delegation_set_connection_trust`（完全信任需 `riskConfirmed`），注册表行为 `ControllerLocal`。
- 外部智能体：在任务目录中运行（子会话继承任务的工作区），提示经主人的审批邮箱处理。
- 测试：core `-- delegation remote_agent tools::implementations::task tool_pipeline external_agent_turn memories::startup bootstrap prompt_builder child_context` 278 通过（含信任存储、拒绝名单、私密文件审批与记住授权仍需审批）；CLI `dispatch::` 等 118 + 1 通过（含按连接读取信任）；`cli_command_contracts delegation` 4 通过：普通连接的模型请求里没有 `USER.md` 内容与 `PagePublish`，`trust-connection --level full --accept-risk` 后下一个任务两者都有，审计分别记录 `normal` 与 `full`；同账号任务两者都有。`cargo test -p openbitfun-product-domains remote_surface` 20 通过；桌面 `remote_workspace_policy` 9 通过；`capabilities:check`、`check:core-boundaries` 通过。
- 偏差：
  - 生成能力目录时发现提交 `09fbba2ef` 新增的 `ai-collaboration` 场景缺少语义归属，导致 `capabilities:generate` 失败；本任务把它归到 `feature.detached-dispatch`（与开放智能体子能力同属）。
  - 本机 Xcode 更新后 xcrun 缓存指向已删除的 SDK；桌面构建脚本需要 `SDKROOT=.../MacOSX.sdk` 才能编译，与本任务无关。
  - 界面中的信任级别设置在任务 4 实现。

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

- [x] AI协作页、信任级别与助理名下的外部任务在界面上可用，已删除功能无残留，均有测试

**Evidence (2026-10-31, Task 4):**
- 开放页（`AgentDelegationConfig.tsx`）按助理列出本机助理（`delegation_list_assistants`），未开放的显示「未开放」，开放时选择本账号设备并确认「助理可以用本设备的登录调用 Claude Code 等外部智能体」；不再有智能体、ID、任务类型与消息工作区字段。已开放的助理可以授权给其他账号（含限制）与收回。旧版的模式与外部智能体记录、无法读取的记录显示为「已不支持」并可删除。
- 外部任务：dispatch 列表条目新增 `delegation`（开放项、助理目录、来源设备或连接、请求方自报信息）与 `pendingPermissionCount`；通过连接查询时两者都被清除。桌面新命令 `delegation_list_tasks`、`delegation_task_status`、`delegation_task_answer`（只接受 `once`/`reject`，只作用于 `delegation-` 任务）以设备主人身份调用同一个 dispatch runner，并解析来源设备名与连接账号名。AI协作页把任务列在对应助理下方，标明来源、状态与待审批数，可以展开查看待审批操作并逐项允许一次或拒绝。左侧导航中助理的会话历史下方同样列出这些外部任务（来源与待审批数），点击进入 AI协作页；多个助理共享一次读取，30 秒刷新。
- 信任级别：连接列表显示每个连接的普通或完全信任；设为完全信任需在确认说明（资料与记忆可读、全部工具可用、记忆由所有助理共享）后再确认；改回普通信任时提示命令与外部智能体只靠审批保护。读取失败时按普通信任显示。
- 审计记录显示信任级别与任务调用的外部智能体，不再显示任务类型；旧条目不显示信任级别。
- Peer Device Mode：开放页、连接列表与导航中的外部任务都不读取本机数据，开放页与连接列表显示不支持。新命令登记为 `ControllerLocal`。
- 测试：`pnpm --dir src/web-ui run test:run src/app/components/NavPanel/sections/workspaces src/infrastructure/config/components/{AgentDelegationConfig,ConnectionsConfig,DelegationAuditView}.test.tsx src/app/scenes/ai-collaboration src/app/scenes/agents` 183 + 3 通过；`type-check:web`、`i18n:audit`、`theme:color-audit:all` 通过，`lint:web` 在改动文件中没有新警告。CLI `dispatch::` 115 通过（主人列表含来源与助理、连接列表清除）；`cli_command_contracts delegation` 4 通过：两账号模拟中主人 `dispatch list` 的任务带 `assistant:workspace`、开放项与来源连接。桌面 `-- delegation_tasks remote_workspace_policy` 11 通过（只处理委托任务、拒绝 `always`）；`capabilities:check`、`check:core-boundaries` 通过。
- 偏差：
  - 外部任务不在 `workspaceSessionView.ts` 中投影为会话条目：它们的会话存放在 dispatch 存储中，并不在助理的会话存储里，投影进会话列表需要伪造会话。改为在助理会话历史下单独列出，点击进入 AI协作页查看与审批。
  - 查看任务只显示状态、待审批操作与错误，不显示完整对话过程。
  - 桌面命令放在新文件 `delegation_tasks_api.rs`，未改 `dispatch_api.rs`（后者是发起方命令）。
  - 新 Rust 模块需要在能力目录中登记语义归属（`delegation_tasks` → `feature.detached-dispatch`）。

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

- [x] RFC 批准条件 1–11 逐项实测并记录（自动化模拟）
- [x] 在模拟客户端中查看 AI协作页与助理名下的外部任务、处理一次审批

**Evidence (2026-10-31, Task 5):**
- `node scripts/dev/sim-devices.mjs --check`（需 `SDKROOT`，见下）：桌面发起方两个场景 `desktop_requester_delegates_to_simulated_devices`、`desktop_requester_delegates_to_other_accounts_over_connections` 通过；模拟设备 `serve_simulated_devices` 通过。
- `cargo test -p openbitfun-cli --test cli_command_contracts delegation` 4 通过（含两账号与同账号模拟）。
- 逐项：
  1. 未开放、未授权不可见；同账号开放后可见；跨账号配对并授权后可见、收回授权后目录为空 —— CLI 两账号与同账号模拟（目录断言）、桌面发起方场景。
  2. 后台委派、结果回到发起会话、重启后补收 —— 桌面发起方场景（F002-06 起覆盖，本次重跑通过）。
  3. 任务以助理身份运行（模型请求含 `SOUL.md` 人设标记，`agentType` 为 `assistant:workspace`），主人列表中位于该助理下并标明来源连接；审批只由目标端回答，连接方回答被拒绝 —— CLI 模拟（`owner_job_list`、`DISPATCH_CONNECTION_ANSWER_REFUSED`）；界面由 Task 4 组件测试覆盖。
  4. 工具与子智能体可用、外部智能体需确认额度后才能开放 —— CLI 模拟中任务执行 `ExecCommand`；`only_assistants_open_and_only_after_confirming_external_agent_use`；外部智能体运行由内核测试覆盖（模拟环境没有真实 ACP 智能体）。
  5. 普通信任：模型请求中没有 `USER.md` 内容与 `PagePublish`；任务读取 `USER.md` 时停下等待主人审批，对方只看到脱敏的 `read` 提示，主人看到文件 —— CLI 两账号模拟（本任务新增）；禁用工具与子智能体的继承由内核单元测试覆盖。
  6. 完全信任后下一个任务含资料与 `PagePublish`、审计记 `full`；改回普通有提示；同账号设备两者都有 —— CLI 模拟。
  7. 写入 `SOUL.md` 即使获批也不生效 —— CLI 同账号模拟；记忆整理跳过任务目录 —— 内核 `memories::startup` 测试。
  8. 只能开放助理、模式与外部智能体开放被拒、旧版编码任务按名拒绝、连接上的工作区准备被拒 —— CLI 模拟与 Task 4 组件测试（旧记录显示已不支持）。
  9. 撤销授权取消进行中的任务、超出并发额度被拒、撤销连接后调用被 Relay 拒绝 —— CLI 两账号模拟；离线与能力缺失由桌面发起方场景覆盖。
  10. 委派回合中再委派被拒（「delegation does not chain across devices」）—— 内核 task 测试。
  11. 审计记录每次跨账号委派及信任级别（`normal`/`full`）；外部智能体字段由 worker 单元测试覆盖 —— CLI 模拟。
- 偏差与问题：
  - 界面实测（解锁后用 cua-driver 操作 A1 桌面补做）：开放「助理」给 A2 并确认外部智能体；授权给 B1，设为完全信任（确认面板、`connection-trust.json` 写入 `full`）后改回普通；以 B1 连接提交一个任务，助理读取 `USER.md` 停下，AI协作页显示来源「sim-b，B1 Desktop」与两项待审批，逐项「允许一次」后任务完成；再开放「助理 2」只授权给 B2，两个助理各自显示开放范围与授权。实测发现并修复了四个问题（随后的提交）：
    - AI协作页只在首次挂载时读取任务，场景标签保持挂载后新任务不出现 —— 改为与侧栏共用的定时读取（读取模块移到 `infrastructure/api/service-api/delegatedTaskReads.ts`）。
    - 侧栏的外部任务只在助理历史展开后出现，而展开按钮只在悬停时可见 —— 历史收起时在助理下方仍列出等待审批的任务。
    - 侧栏按路径匹配任务所属助理，运行器记录的 `/var/...` 与应用的 `/private/var/...` 不相等导致不显示 —— 改为按 `assistant:<目录名>` 匹配。
    - 外部任务行沿用 26px 固定行高被截断、未缩进；完全信任确认说明被挤在窄列 —— 修正样式与行布局。
  - 遗留：提示中的操作名直接显示为 `read`、`external_directory`；侧栏在主人审批后最多 30 秒才刷新等待数。
  - 重新构建桌面时外观契约审计发现 Task 4 的导航外部任务使用了未声明的外观部件，已修复（提交 `9273ca326`）。
  - 本机 Xcode 更新后 xcrun 缓存指向已删除的 SDK，构建与模拟需要 `SDKROOT=.../MacOSX.sdk`；可运行 `xcrun --kill-cache` 修复。

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/06-cross-account-delegation.md`
