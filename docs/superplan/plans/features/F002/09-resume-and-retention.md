---
id: "F002-09"
title: "AI协作：接回以前的外部任务、按授权保留会话，并删除兼容代码"
type: "feature"
status: "complete"
summary: "发起设备可以在新的对话中接回自己以前派给远程助理的任务并延续原会话；目标端按授权配置外部任务会话的保留时长（默认 7 天）；删除 AI协作对它更早开发版本的兼容代码。"
source: "docs/superplan/human/features.md"
created: "2026-11-01"
order: 9
depends_on: ["F002-08"]
parent: "F002"
---

# AI协作：接回以前的外部任务、按授权保留会话，并删除兼容代码 Plan

**需求基准：** RFC `docs/superplan/rfcs/F002.md` 第 5 版的「AI协作需求（基准）」、D4（会话保留）、D12（接回以前的外部任务）与「兼容性、安全与回滚」中的「未发布功能」。

**Goal:** 用户在新的对话里能接着用远程助理以前处理过的会话，主人决定这些会话保留多久，AI协作代码里不再有为更早开发版本保留的分支。

**Scope:**
- **删除兼容代码**：AI协作只按当前格式读写，删除以下内容及其测试、界面与文案：
  - 第 3 版开放项（智能体模式、外部智能体）的「已不支持」处理：`NO_LONGER_SUPPORTED`、按条目原样保存再逐条解析的开放清单、界面中的「已不支持 / 无法读取」条目与删除入口、CLI 中对应的列出与删除说明。开放清单改为整体按类型读写，读不了时明确报错，不删除文件。
  - 任务类型：只有消息任务，删除 `DelegationTaskKind`、开放项的 `task_kinds`、授权与准入中的任务类型参数、协议中的 `DispatchDelegationTaskKind`（含 `Coding`、`Unknown`）与「目标只接受消息任务」的拒绝。
  - 为未知值保留的 `#[serde(other)] Unknown`：审计事件、受阻原因、远程任务状态、信任级别等 AI协作类型；以及 TypeScript 中「未知值保留为字符串」的类型放宽。
  - 只为旧记录存在的默认值与测试：旧连接授权缺少额度、旧出站记录缺少游标、旧描述缺少账号、旧连接记录缺少可选字段、信任级别缺失视为普通等。
  - 保留：对已发布版本的能力协商与「设备需要更新」（已发布的 OpenBitFun 不声明 AI协作能力）；不属于 AI协作的 Dispatch、ACP、工作区兼容代码。
- **按授权保留会话（目标端）**：
  - 授权带会话保留天数：同账号设备一项（对本账号全部被授权设备生效），每个跨账号连接授权一项；默认 7 天，可选 1、7、30、90 天。
  - 目标端清理时，经授权进入的外部任务按其授权的保留天数、从最后一个回合结束起计算；授权已删除时按 7 天。不经授权的普通 Dispatch 任务仍为 30 天。修改保留天数对已有任务生效。
  - AI协作页的授权行可以设置保留天数；CLI 授权命令带对应参数。
- **接回以前的外部任务（发起方）**：
  - 发起设备列出以前派给某个远程助理的任务（第一次请求的摘要、开始时间、状态），来源是本机出站记录；原对话被删除后不再列出。
  - 发起方助理在当前对话中接回其中一个：作为原任务的新回合提交（`continue`），结果经 `AgentWait` 回到当前对话；目标端仍是该助理名下同一个会话。
  - 一个任务同时只属于一个对话：接回后原对话再追问时明确告知「任务已在另一个对话中继续」；任务运行或排队时不能接回，明确告知原因。
  - 目标端拒绝（不是该连接发起的任务、授权已撤销、任务已被清理或删除）时，接回明确失败并给出原因，不悄悄新建任务。

**Non-Goals:**
- 接回其他设备、其他连接或其他人发起的任务；目标主人把会话转交给别人。
- 发起方界面中的以前任务浏览列表（由发起方助理按用户的要求查找与接回）。
- 目标端新增按会话浏览的协议动词。
- 不经授权的普通 Dispatch 任务的保留策略、ACP 与工作区的兼容代码。

**Architecture:**
- **兼容代码**：AI协作的所有代码只在本分支，没有发布过（`origin/main` 中没有 `OpenAgentStore`、`DelegationTrust`、`DispatchDelegationTaskKind` 等）。删除后协议版本不变，双方都必须是本分支的新构建；已发布版本仍由能力协商挡在外面。
- **会话保留**：保留天数存于开放清单的授权中（`OpenAgent` 的同账号授权与 `ConnectionGrant`），由目标端 Dispatch 清理时按任务记录中的开放项与来源（设备或连接）查找。清理沿用现有的保留锁与每日清理节奏，删除任务目录即删除该外部任务会话。
- **接回**：
  - 列出：发起方协调存储中带 `remote_job` 的后台任务按远程助理地址查询，跨对话，只读本机记录；摘要取该任务第一次请求的开头。
  - 接回：`Task` 增加引用以前任务的参数；有该参数时在当前对话中建立新的子会话，链接到原远程任务，并通过已有的 `continue` 提交；接回前用 `status` 确认任务已结束。
  - 归属：协调存储记录每个远程任务当前所属的对话；不属于当前对话的子会话追问时被拒绝。这是本机的记录，不需要目标端参与。
  - 目标端已按连接只允许继续自己发起的任务、并在继续时重新检查授权（`dispatch/connection.rs`），本计划不改这部分。
- **远程场景**：接回只在发起设备本机生效；Peer Device Mode 中出站记录属于对端主机（D11），由对端主机列出与接回。

**Baseline:**
- 同一对话中的追问已经延续原会话：`remote_agent_turn.rs` 按子会话找到出站记录后调用 `continue_job`；`AgentSendInput` 只能找到当前对话自己的子会话（`ensure_subagent_session_loaded_for_reuse`）。
- 目标端已结束的 Dispatch 任务统一保留 30 天（`dispatch/store.rs` 的 `TERMINAL_JOB_RETENTION_DAYS`），每天最多清理一次。
- 开放清单按条目原样保存，读不了或不是助理的条目显示为「已不支持 / 无法读取」；授权带任务类型。

**Exit Criteria:**
- 代码中不再有 AI协作的旧格式、旧类型与未知值兼容处理；对已发布版本的能力协商仍在。
- 主人为授权设置保留天数（未设置时为 7 天）；到期的外部任务及其会话被清理，之后接回明确失败。
- 发起方在新的对话中接回以前的任务：回答延续原会话的上下文，目标端仍是同一个会话；原对话再追问被明确告知已转走；运行中不能接回；其他连接、撤销后重新配对的连接，或已清理的任务，接回明确失败（CLI 与桌面两账号模拟）。

## Task 1: 删除 AI协作的兼容代码

**Outcome:** AI协作的存储、协议与界面只认当前格式，行为不变，代码与测试中不再有为更早开发版本保留的分支。

**Files:**
- Modify: `src/crates/assembly/core/src/service/delegation.rs`、`src/crates/assembly/core/src/service/delegation_audit.rs`
- Modify: `src/crates/contracts/runtime-ports/src/remote_agent.rs`、`src/crates/contracts/runtime-ports/tests/remote_agent_port_contracts.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/remote_agent_turn.rs`、`remote_agent_turn/tests.rs`
- Modify: `src/crates/assembly/core/src/service/dispatch/remote_agents.rs`
- Modify: `src/crates/services/services-integrations/src/remote_connect/connections.rs`、`connections/tests.rs`
- Modify: `src/apps/cli/src/dispatch/{protocol,connection,connection_guard,mod}.rs`、`src/apps/cli/src/delegation_cli.rs`、`src/apps/cli/tests/cli_command_contracts/delegation_simulation.rs`
- Modify: `src/apps/desktop/src/api/delegation_api.rs`、`src/apps/desktop/src/api/remote_agent_delegation/simulation_tests.rs`
- Modify: `src/web-ui/src/infrastructure/api/service-api/{DelegationAPI,ConnectionsAPI}.ts`、`src/web-ui/src/infrastructure/config/components/AgentDelegationConfig.{tsx,test.tsx}`、`src/web-ui/src/locales/*/settings/runtime.json`
- Modify: `docs/architecture/detached-task-dispatch.md`（删除对旧开放项与编码任务拒绝的说明）

**Change Map:**
- `delegation.rs`: 开放清单整体按类型读写；删除 `NO_LONGER_SUPPORTED`、`OpenAgentRecord` 的「不支持 / 无法读取」、`DelegationTaskKind` 与 `task_kinds`；`authorize`、`authorize_connection` 去掉任务类型参数；删除只为旧记录存在的默认值与测试。
- `delegation_audit.rs`、`remote_agent.rs`: 删除 `#[serde(other)] Unknown` 及其分支和测试。
- `dispatch/protocol.rs`、`connection.rs`、`connection_guard.rs`、`mod.rs`: 删除 `DispatchDelegationTaskKind` 与编码任务拒绝。
- Web UI：删除「已不支持」条目与删除入口、对应文案与测试；类型不再为未知值放宽。

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib delegation`
- `cargo test --locked -p openbitfun-runtime-ports --features remote-agent-port`
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib -- remote_agent background_outcomes agent_wait`
- `cargo test --locked -p openbitfun-cli --bin openbitfun dispatch::`；`cargo test --locked -p openbitfun-cli --test cli_command_contracts delegation`
- `cargo test --locked -p openbitfun-services-integrations remote_connect::connections`
- `pnpm --dir src/web-ui run test:run src/infrastructure/config/components`；`pnpm run type-check:web`；`pnpm run i18n:audit`
- 检查：在 AI协作相关文件中搜索 `serde(other)`、`legacy`、`earlier build`、`no longer supported`、`DelegationTaskKind`、`Coding`，不再出现兼容用法

- [x] AI协作的兼容代码删除，现有行为与测试保持通过
  - 证据（2026-11-01）：
    - 删除：`DelegationTaskKind`、开放项 `task_kinds`、协议 `DispatchDelegationTaskKind` 与编码任务拒绝；`NO_LONGER_SUPPORTED`、`OpenAgentRecord` 与逐条原样保存（开放清单改为 `Vec<OpenAgent>`，含非助理条目时整体报错、不改写文件）；审计 `Unknown` 事件与 `taskKind`、可缺失的信任与对端字段；`RemoteAgentJobState`、`RemoteAgentHoldReason`、`DispatchHoldReason` 的 `Unknown`；连接信任未知值；出站记录游标默认值；目录对旧目标的非助理过滤与 `taskKinds`；Web UI「已不支持 / 无法读取」区块、相关文案与宽松类型；对应的旧记录测试。`detached-task-dispatch.md` 的委派一节按当前设计重写。
    - 保留：对已发布版本的能力协商与「设备需要更新」；连接文件与审计日志对损坏条目的保留；普通 Dispatch 的兼容。
    - core（`delegation remote_agent background_outcomes agent_wait`）83 项、runtime-ports 8 + 6 项、`remote_connect::connections` 10 项、CLI `dispatch::` 115 + 1 项、CLI 合约 `delegation` 4 项、Web UI `config/components` 538 项通过；`type-check:web`、`i18n:audit` 通过；桌面 `cargo check --lib --tests` 通过。

## Task 2: 按授权保留外部任务会话

**Outcome:** 主人在授权时设置会话保留天数（默认 7 天），目标端按它清理外部任务及其会话。

**Files:**
- Modify: `src/crates/assembly/core/src/service/delegation.rs`
- Modify: `src/apps/cli/src/dispatch/store.rs`、`src/apps/cli/src/delegation_cli.rs`
- Modify: `src/apps/desktop/src/api/delegation_api.rs`
- Modify: `src/web-ui/src/infrastructure/api/service-api/DelegationAPI.ts`、`src/web-ui/src/infrastructure/config/components/AgentDelegationConfig.{tsx,test.tsx}`、`src/web-ui/src/locales/*/settings/runtime.json`
- Test: 上述模块测试

**Change Map:**
- `delegation.rs`: 同账号授权与 `ConnectionGrant` 带保留天数（1、7、30、90，默认 7）；按任务的开放项与来源查询保留天数。
- `dispatch/store.rs`: 清理已结束任务时，经授权进入的任务使用其授权的保留天数，授权已删除时为 7 天；其他任务仍为 30 天。
- `delegation_cli.rs`、`delegation_api.rs`、AI协作页：设置与显示保留天数。

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib delegation`
- `cargo test --locked -p openbitfun-cli --bin openbitfun dispatch::`（新增：按授权天数清理、授权删除后按 7 天、修改天数对已有任务生效、普通任务仍为 30 天）
- `pnpm --dir src/web-ui run test:run src/infrastructure/config/components`；`pnpm run type-check:web`；`pnpm run lint:web`；`pnpm run i18n:audit`
- `cargo check -p openbitfun-desktop --lib`

- [x] 授权可设置保留天数，清理按授权执行，均有测试
  - 证据（2026-11-01）：
    - `ConnectionLimits.session_retention_days`（每个连接授权）与 `OpenAgent.session_retention_days`（同账号设备），可选 1、7、30、90 天，未设置为 7 天；`session_retention_days` 按任务来源查找，授权或开放项已删除时为 7 天。
    - 目标端清理（`DelegatedRetention`）：经授权的任务按其授权天数清理，并删除该任务的独立目录与会话目录（`project_runtime_root`）；开放清单读不了时本轮不清理委派任务；普通任务仍为 30 天。
    - CLI `delegation open|grant-connection --retention-days`，列表显示「sessions kept N day(s)」；AI协作页「限制与保留」中选择保留天数。
    - 测试：core `delegation` 31 项（新增 `sessions_are_kept_as_long_as_each_grant_says`）；CLI `dispatch::` 117 + 1 项（新增 `delegated_tasks_are_kept_as_long_as_their_grant_says`）；CLI 合约 `delegation` 4 项（`--retention-days 30` 生效、`2` 被拒）；Web UI `config/components` 538 项（保留天数选项与保存）；`type-check:web`、`i18n:audit`、`lint:web`（无新增告警）通过。
    - 同时修复：Dispatch 权限邮箱在写入临时文件时被并发读取会整体报错（`origin/main` 同样存在），读取时跳过尚未发布的 `.*.tmp`，新增 `a_permission_being_written_does_not_break_the_mailbox`；合约测试按 `read` 提示查找，不依赖两条提示的先后。跨账号合约测试连续 9 次通过。

## Task 3: 发起方接回以前的外部任务

**Outcome:** 发起方助理能列出以前派给某个远程助理的任务，并在当前对话中接回其中一个，延续目标端原来的会话。

**Files:**
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordination_store.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/remote_agent_turn.rs`、`remote_agent_turn/tests.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/{schema,input,execution}.rs`、`task/tests.rs`
- Create: 列出以前远程任务的工具（`src/crates/assembly/core/src/agentic/tools/implementations/` 下，延迟暴露，只在有可用远程助理时出现）
- Modify: `src/apps/desktop/src/api/remote_agent_delegation/simulation_tests.rs`
- Modify: `docs/architecture/detached-task-dispatch.md`（接回与归属）

**Change Map:**
- `coordination_store.rs`: 跨对话按远程助理列出出站记录（摘要、开始时间、状态）；记录远程任务当前所属的对话并在接回时转移。
- `task/*`: `Task` 接受以前任务的引用；校验它属于本机出站记录、已结束，然后在当前对话建立链接到原任务的子会话。
- `remote_agent_turn.rs`: 子会话不再拥有其远程任务时拒绝追问并说明去向；接回的回合走 `continue_job`，目标端拒绝时如实报告。

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib -- remote_agent coordination_store tools::implementations::task`
- `cargo test -p openbitfun-desktop --lib remote_agent_delegation`（两账号模拟：新对话接回后回答延续上下文、目标端仍是同一会话；原对话追问被告知已转走；运行中不能接回；其他连接与撤销后重新配对接回失败；任务被清理后接回失败）
- `node scripts/dev/sim-devices.mjs --check`

- [x] 发起方可列出并接回以前的任务，归属转移与各类拒绝均有测试
  - 证据（2026-11-01）：
    - 列出方式（与计划不同，已记录）：未新增延迟工具。新工具需要改动约 15 处工具注册表与生成目录；改为在 `Task`/`AgentSpawn` 的可用智能体目录中，每个远程助理下列出本机最近派给它的 5 个任务（编号、开始与最后回合日期、第一次请求摘要、是否仍在运行），满足 RFC D12「从本机出站记录列出」。
    - `CoordinationStore::outbound_remote_tasks` 跨对话列出出站记录；`RemoteJobLink` 增加 `agentType` 与 `summary`；任务归属 = 带该任务链接的最新后台任务（`remote_agent_turn/earlier_tasks.rs`）。
    - `AgentSpawn`/`Task` 新增 `resume_task_id`（只用于远程智能体的新建；分叉、追问、取消与非远程类型拒绝）。工具先校验任务存在、属于该助理、本机未在运行；回合再观察一次目标端，未结束则拒绝，然后 `continue_job`，归属转到当前对话；原对话追问报「已在另一个对话中继续」；目标端拒绝原样报告。
    - core 测试（`remote_agent coordination_store tools::implementations::task agent_wait background_outcomes delegation`）193 项通过，新增 `a_later_conversation_resumes_an_earlier_task_and_takes_it_over`（未知任务、目标端仍运行、接回成功且从原游标继续、归属转移、原对话被拒、新对话可继续追问）、`a_task_still_running_here_is_not_resumed`、`summaries_are_one_short_line`、`an_earlier_remote_task_is_resumed_only_by_a_fresh_spawn`。
    - 文档：`docs/architecture/detached-task-dispatch.md` 增加接回、归属与按授权保留。
    - 两账号模拟 `node scripts/dev/sim-devices.mjs --check` 通过（2 项）：B1 上已结束的任务被接回，同一任务继续并成功，审计多一条该任务的 `finished`；B1 已不存在的任务接回失败；B2 撤销连接后接回失败。

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/07-open-assistants.md`
- `docs/superplan/plans/features/F002/08-waiting-and-queueing.md`
- `docs/architecture/detached-task-dispatch.md`
