---
id: "F001-04"
title: "外部智能体委派的远程场景"
type: "feature"
status: "in_progress"
summary: "外部智能体委派在 SSH 远程工作区、手机端与 IM 远程控制、Peer Device Mode 下可用，Detached Dispatch 中明确报不支持。"
source: "docs/superplan/human/features.md"
created: "2026-10-29"
order: 4
depends_on: ["F001-02", "F001-03"]
parent: "F001"
---

# 外部智能体委派的远程场景 Plan

**Goal:** 外部智能体委派在仓库的四种远程场景中，要么正常工作，要么给出明确的不支持提示，不会悄悄回退到本地执行。

**Scope:**
- **SSH 远程工作区**：子会话通过现有的 ACP 远程连接路径在远程主机上运行；远程主机缺少外部智能体或探测失败时，列表与委派都返回明确原因。
- **远程控制**：手机端与 IM 机器人通过 `RemoteCommand` 协议和机器人命令路由看到委派状态，能取消委派、回答外部智能体的权限请求。不支持的操作返回明确提示。
- **Peer Device Mode**：委派在对端主机上运行，并使用对端的登录；CLI peer host 处理与委派相关的命令。
- **Detached Dispatch**：Dispatch 工作进程中委派给 `acp:` 类型时明确失败，并给出原因。
- 产品操作注册表：新增或变更的桌面命令都有对应行，不增加 `Unaudited`。

**Non-Goals:**
- 不在 Dispatch 工作进程中启用外部智能体（F002 第 2 阶段）。
- 不做跨设备委派（F002）。

**Architecture:**
- 远程控制走内核与权限邮箱，不依赖桌面窗口。
- 远程工作区路径一律按 POSIX 处理，不用宿主的 `std::path` 语义拆分或拼接。

**Baseline:**
- 独立 ACP 会话的流式路径已支持 `remote_connection_id`（`acp_client_api.rs`）；旧工具 `AcpAgentTool` 在远程上下文中不可用（`tool.rs`）。
- 现有能力清单包含「在 SSH 远程工作区探测可用外部智能体」和「远程回答外部智能体权限请求」。
- ACP 相关命令在注册表中为 `Unaudited`（`remote_surface/table.rs`）。

**Exit Criteria:**
- 在 SSH 远程工作区中委派给 Claude Code，任务在远程主机上执行；远程主机缺少 Claude Code 时返回明确错误，不在本地执行。
- 手机端能看到后台委派的状态，能取消，也能回答其权限请求。
- Peer 模式下委派在对端执行。
- Dispatch 任务中委派 `acp:` 类型时明确失败。
- 注册表和闭包测试通过，`Unaudited` 数量没有增加。

## Task 1: SSH 远程工作区执行与不可用原因

**Outcome:** ACP 子会话在远程工作区中经远程连接执行；不可用时明确报错。

**Files:**
- Modify: `src/crates/contracts/runtime-ports/src/external_agent.rs`
- Modify: `src/crates/interfaces/acp/src/client/executor.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/external_agent_turn.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordinator.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/acp_agents.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/query.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/mod.rs`
- Modify: `src/apps/desktop/src/api/subagent_api.rs`
- Modify: `src/apps/cli/src/peer_host/commands/tools.rs`
- Test: `src/crates/interfaces/acp/src/client/executor.rs`
- Test: `src/crates/assembly/core/src/agentic/coordination/external_agent_turn/tests.rs`
- Test: `src/crates/assembly/core/src/agentic/agents/registry/tests.rs`

**Change Map:**
- `external_agent.rs`：`list_agents` 增加 `remote_connection_id`，带连接时探测结果描述远程主机
- `executor.rs`：远程探测（含首次无缓存时强制探测）抽成 `requirement_probes`，`availability` 与 `list_agents` 共用；远程主机缺少探测结果时报告「远程主机上未找到」
- `external_agent_turn.rs`、`coordinator.rs`：`external_agent_turn_context` 从子会话的工作区绑定取连接标识与 POSIX 根路径；远程会话解析不到连接时直接失败，不在本机执行
- `acp_agents.rs`、`query.rs`：新增 `ExternalAgentHost { Local, Remote, Unsupported }` 与 `get_subagents_for_management`；ACP 条目只进入总览列表，并按工作区所在主机探测
- `task/mod.rs`：Task 目录中的 ACP 条目不再跟随 external-sources 开关，远程工作区也会列出；Swarm 判定与准入一致
- `subagent_api.rs`、`tools.rs`：桌面端与 CLI peer 的 `list_subagents` 传入工作区所在主机；按模式的可管理列表只保留本地可开关的智能体

**Deviations:**
- `directory.rs`、`validation.rs` 未改动：目录能力在 `executor.rs`（见 F001-03），远程不可用原因由准入时的 `availability` 给出，无需在 `validation.rs` 另加分支。
- F001-03 中按模式的可管理列表（`list_manageable_subagents`）也会带出 ACP 条目；本任务收窄为只在总览 `list_subagents` 中出现，因为按模式列表只用于开关，而 ACP 条目不能在那里开关。

**Verification:**
- `cargo +1.95.0 test -p openbitfun-acp --no-default-features --features client,openbitfun-core/git --lib client::`
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::coordination::external_agent_turn`
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::tools::implementations::task`
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::agents::registry`
- `cargo +1.95.0 test --locked -p openbitfun-runtime-ports --no-default-features --features external-agent-port --test external_agent_port_contracts`
- `cargo +1.95.0 test -p openbitfun-cli --bin openbitfun peer_host::`
- `cargo +1.95.0 test -p openbitfun-desktop --lib api::subagent_api`

- [x] 远程会话透传 SSH 连接标识与 POSIX 根路径；远程会话缺少连接时报错且不在本机执行；远程主机缺少探测时报告原因（external_agent_turn 14、acp `client::` 94、task 90、registry 58、端口契约 7、CLI `peer_host::` 105、desktop 1 passed）。仅单元测试，尚未在真实 SSH 主机上运行。

## Task 2: 远程控制、Peer 与 Dispatch

**Outcome:** 手机端、IM 和 CLI peer host 都能处理委派状态、取消和权限回答；Dispatch 中明确报不支持。

**Files:**
- Modify: `src/crates/contracts/runtime-ports/src/external_agent.rs`
- Create: `src/crates/interfaces/acp/src/client/permission_broker.rs`
- Modify: `src/crates/interfaces/acp/src/client/manager.rs`
- Modify: `src/crates/interfaces/acp/src/client/executor.rs`
- Modify: `src/crates/interfaces/acp/src/client/mod.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/external_agent_turn.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordinator.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/pipeline/tool_pipeline.rs`
- Modify: `src/crates/assembly/core/src/service_agent_runtime.rs`
- Modify: `src/mobile-web/src/components/PermissionMailbox.tsx`
- Modify: `src/apps/cli/src/agent/agentic_system.rs`
- Modify: `src/apps/cli/src/dispatch/worker.rs`
- Test: `src/crates/contracts/runtime-ports/tests/external_agent_port_contracts.rs`
- Test: `src/crates/interfaces/acp/src/client/permission_broker.rs`
- Test: `src/crates/assembly/core/src/agentic/coordination/external_agent_turn/tests.rs`
- Test: `src/crates/assembly/core/src/service_agent_runtime.rs`
- Test: `src/mobile-web/tests/interaction-mailbox-browser.test.mjs`
- Test: `src/apps/cli/src/agent/agentic_system.rs`

**Change Map:**
- `external_agent.rs`：`ExternalAgentEventSink::request_permission`，以及权限请求、种类（未知种类按 `other` 解析）与决定的 DTO；默认实现返回 `NotAvailable`
- `permission_broker.rs`、`manager.rs`、`executor.rs`：委派回合运行期间为其 OpenBitFun 会话登记权限中转；该会话的 ACP 权限请求交给回合，不再发出仅桌面可见的 `backend-event-acppermissionrequest`，也不使用独立会话的 600 秒超时；回合无法中转时回退到原有路径。独立 ACP 会话不受影响
- `external_agent_turn.rs`、`coordinator.rs`、`tool_pipeline.rs`：内核把请求登记进与原生工具相同的权限邮箱，归属子会话并带父 Task 的委派上下文（动作映射为 `read`、`edit`、`bash`、`webfetch`、`custom_tool`）；回合取消时撤回待答请求；带修改输入的批准按拒绝处理，因为外部智能体的输入不由 OpenBitFun 管理
- `service_agent_runtime.rs`：远程轮询（手机端与 IM 机器人共用）同步父会话的待答权限时，包含委派子会话发起的请求，与 `session_interaction_snapshot` 一致；`ConfirmTool` 按请求 id 或工具调用 id 应答
- `PermissionMailbox.tsx`：手机端按 `delegation.parentSessionId` 展示子智能体请求；外部智能体请求不显示「修改后批准」
- `agentic_system.rs`、`worker.rs`：Dispatch 工作进程登记一个始终返回 `UnsupportedContext` 的执行器，委派 `acp:` 时给出「Detached Dispatch 中不可用」的原因；目录中不列出外部智能体

**Deviations:**
- 没有新增 `RemoteCommand` 或桌面命令：远程控制复用已有的权限邮箱、`ConfirmTool`、`cancel_dialog_turn` 与 `get_session_interaction_mailbox`。因此注册表无新增行，`Unaudited` 数量不变（独立 ACP 命令仍在冻结的待审清单中）。
- `peer_host/commands/mod.rs`、`validation.rs`、`table.rs` 未改动：Peer 模式下委派在对端内核执行，所需命令（`list_subagents`、`cancel_dialog_turn`、`respond_permission`、`get_session_interaction_mailbox`）在 CLI peer host 中已是 `HANDLED`；Dispatch 的拒绝在 CLI 进程装配处实现。
- 修复了手机端与远程轮询只按 `sessionId` 过滤权限请求的问题。该问题同样影响原生子智能体的权限请求。
- 桌面端委派的 ACP 权限现在出现在父会话的审批条中（桌面端已有按 `delegation.parentSessionId` 的路由），不再以 ACP 权限卡片的形式出现。

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-runtime-ports --no-default-features --features external-agent-port --test external_agent_port_contracts`
- `cargo +1.95.0 test -p openbitfun-acp --no-default-features --features client,openbitfun-core/git --lib client::`
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::coordination::external_agent_turn`
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features remote-connect --lib delegated_permission_tests`
- `cargo +1.95.0 test -p openbitfun-cli --bin openbitfun -- peer_host:: agent::agentic_system dispatch::`
- `cargo +1.95.0 test -p openbitfun-product-domains --no-default-features remote_surface`
- `cargo +1.95.0 check -p openbitfun-desktop`
- `pnpm --dir src/mobile-web run test:interaction-mailbox`
- `pnpm run capabilities:generate && pnpm run capabilities:check`
- `pnpm run type-check:mobile-web`
- `pnpm run check:core-boundaries`

- [x] 端口契约 9、acp `client::` 96、external_agent_turn 17（含权限进入父会话邮箱、取消时撤回）、远程轮询过滤 1、CLI 192 + Dispatch 拒绝 1、注册表 20 passed；手机端邮箱浏览器测试 2 passed；能力清单生成无变化、检查通过；手机端类型检查与边界检查通过；桌面端编译通过

**Evidence notes:**
- ACP 管理器中的中转登记与回退路径只有纯函数与内核侧测试；没有在真实 ACP 进程上验证权限往返。
- IM 机器人复用远程轮询与 `ConfirmTool`，本任务没有单独的机器人测试。

## Task 3: 远程场景实测记录

**Outcome:** 每种远程场景都有实际运行的证据，记录在本计划中。

**Files:**
- Modify: `docs/superplan/plans/features/F001/04-remote-scenarios.md`

**Verification:**
- 手动：在 SSH 远程工作区、手机端远程控制、Peer 模式下各执行一次后台委派与取消，记录结果；在 Dispatch 任务中验证明确失败的提示

- [ ] 四种场景的实测结果已记录（Dispatch 与本机基线已实测；SSH、手机端、Peer 待实测）

**实测记录（2026-10-29，本机 macOS，`target/debug/openbitfun`，真实 Claude Code `claude-agent-acp@0.99.0`）：**
- 本机基线：`exec` 中 `AgentSpawn(acp:claude-code)` 在后台启动子会话，Claude Code 用自己的 `Read` 工具读出测试文件内容，结果经 `AgentWait` 回到父会话。
- 取消：父会话停 15 秒后 `AgentControl interrupt`，返回 `interrupted_background_tasks: 1`；日志中向 Claude Code 会话发送了 ACP `session/cancel`；`AgentWait` 返回 `outcome: cancelled`。
- 权限往返（隔离配置目录 `OPENBITFUN_USER_ROOT`，claude-code 设为 `ask`）：Claude Code 的 Write 请求进入内核权限管理器，动作为 `edit`，带委派上下文（`acp:claude-code`、父会话、父任务）。`exec` 不带 `--auto` 时由 CLI 拒绝并报 `action-required`，文件未写入；带 `--auto` 时按会话自动批准，文件写入。
- Detached Dispatch：真实提交任务，`AgentSpawn(acp:claude-code)` 立即失败并给出「External agents are not available in Detached Dispatch jobs…」，任务本身正常结束，没有启动 Claude Code 进程。
- 每次运行后都没有残留的 `claude-agent-acp` 进程。
- 实测发现并修复两处缺陷：一是模型实际看到的智能体列表来自每轮快照（`skill_agent_snapshot.rs`），之前没有加入 `acp:` 条目，模型因此看不到 Claude Code；现在与 `AgentSpawn` 目录共用同一道准入检查。二是自动批准模式的会话中，外部智能体的权限请求没有人回答，会一直挂起；现在按会话权限模式自动批准。
- 未实测：SSH 远程工作区（本机没有 sshd，Docker 未运行；`~/.ssh/config` 中的主机是用户的真实服务器，未经同意不使用）；手机端远程控制与 Peer 模式（需要 relay 与第二台设备）。这三项目前只有单元测试证据。

## References
- `docs/superplan/rfcs/F001.md`
- `docs/superplan/plans/features/F001/02-task-delegation.md`
- `docs/superplan/plans/features/F001/03-agent-list.md`
- `docs/architecture/remote-surface-contract.md`
- `src/crates/services/services-integrations/AGENTS.md`
- `src/mobile-web/AGENTS.md`
