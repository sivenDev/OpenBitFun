---
id: "F001-02"
title: "通过 Task 后台委派外部智能体"
type: "feature"
status: "complete"
summary: "外部智能体作为 Task 的子智能体类型 acp:<client_id>，支持后台运行、AgentWait、追加指令、级联取消与可打开的子会话。"
source: "docs/superplan/human/features.md"
created: "2026-10-29"
order: 2
depends_on: ["F001-01"]
parent: "F001"
---

# 通过 Task 后台委派外部智能体 Plan

**Goal:** 助理把任务交给 Claude Code 后不被阻塞，结果用原生子智能体相同的方式取回，过程可见、可取消。

**Scope:**
- `Task` 接受 `subagent_type: "acp:<client_id>"`，仅当该客户端已启用、`subagent.enabled` 为真且执行器可用时才允许；同时传 `fork_context` 时明确报错。
- 子会话的每一轮由执行器（F001-01）执行，而不是模型循环。支持前台与后台运行、`AgentWait`、`send_input`（作为追加输入）、`cancel`、超时。
- `AgentWait` 的结果包含最终回答和本次改动文件的摘要。子会话持久化时带父会话关联，复用 ACP 会话记录形态。
- 取消级联：父回合取消时，所有进行中的 ACP 子会话都发出 ACP 取消；修正桌面端 `cancel_dialog_turn` 命中 ACP 会话就提前返回的问题，让父回合与子会话的取消各自生效。
- 子会话的权限请求带父、子会话标识进入权限邮箱；后台运行等待审批时，父会话可见。
- 外部智能体出现在 `Task` 可选子智能体类型的描述中；`acp__<client>__prompt` 不再暴露给模型，但保留历史工具卡片的渲染与解析。
- Web UI：父会话的 `Task` 工具卡片可以打开 ACP 子会话；后台运行中、等待审批、已取消几种状态都有展示。

**Non-Goals:**
- 不做智能体列表展示（F001-03）。
- 不做远程工作区、远程控制或 Peer 的专门适配（F001-04）。
- 不让外部智能体反向委派。
- 不新增按次数的并发上限。

**Architecture:**
- 分支点在内核：调度器或执行引擎根据子会话的 `agent_type` 前缀 `acp:` 选择执行器路径，后台记录、取消令牌、结果交付、`AgentWait` 都沿用现有子智能体机制。
- 外部智能体的工具策略由外部智能体自己负责；OpenBitFun 不向它注入工具，也不经过 OpenBitFun 的工具管线。
- 兼容：历史会话里的 `acp__…__prompt` 调用通过保留的渲染与结果解析正常回放；`acpClients` 配置结构不变。

**Baseline:**
- 后台子智能体：`Task` → `coordinator.start_background_subagent`（`coordinator.rs:13459`）→ `scheduler.submit_hidden_subagent` → 执行引擎；结果由 `AgentWait` 取回。
- 动态工具合并：`merge_dynamic_acp_tools`（`registry/support.rs:168`）把 `acp__` 工具并入非 minimal 模式。
- 桌面端取消：`cancel_dialog_turn`（`agentic_api.rs:3208`）先尝试取消 ACP 会话，命中后直接返回。

**Exit Criteria:**
- 在助理中以 `run_in_background: true` 委派给 `acp:claude-code` 后，立即返回 `bg_task_id`，助理可以继续；`AgentWait` 取回回答与改动文件摘要。
- 父回合取消或单独 `cancel` 时，ACP 外部会话收到取消，子会话进入已取消状态，父会话收到取消结果。
- 后台子会话的权限请求出现在父会话并可以回答，回答后继续执行。
- 新会话的模型可见工具中不再有 `acp__…__prompt`；含旧调用的历史会话正常显示。
- 独立 ACP 会话入口的行为不变。

## Task 1: 内核支持 ACP 子智能体类型与执行分支

**Outcome:** `Task` 可以把 `acp:<client_id>` 作为子智能体类型，子会话的回合经执行器运行，前台、后台、`AgentWait`、`send_input`、超时都可用。

**Files:**
- Create: `src/crates/assembly/core/src/agentic/coordination/external_agent_turn.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/mod.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordinator.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/execution.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/schema.rs`
- Test: `src/crates/assembly/core/src/agentic/coordination/external_agent_turn/tests.rs`
- Test: `src/crates/assembly/core/src/agentic/tools/implementations/task/tests.rs`

**Change Map:**
- `external_agent_turn.rs`：经执行器运行一轮回合，把执行器事件投影为子会话的模型轮次、文本、思考、工具事件；子会话取消令牌触发执行器取消；结果整形为 `ExecutionResult`，使完成、超时、取消、持久化路径原样适用
- `coordinator.rs`：分支点在 `execute_hidden_subagent_internal` 里唯一的回合任务——`acp:` 类型走执行器，其余不变；前台、后台（`start_background_subagent` 进入同一函数）、`send_input` 共用
- `execution.rs`：`acp:` 类型绕过本地子智能体目录，按执行器可用性准入；拒绝 `model_id`/`inherit` 覆盖与 Ultra Swarm 内委派
- `schema.rs`：说明 `acp:<client_id>` 类型；目录中的条目由 F001-03 的目录提供者加入
- 偏差：`validation.rs`、`scheduler.rs`、`execution_engine.rs` 无需修改——现有输入解析已拒绝 `subagent_type` 与 `fork_context=true` 同用，回合分支只需一处

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::coordination::external_agent_turn`
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::tools::implementations::task`

- [x] 假执行器测试覆盖：前台完成并流式输出、同一子会话的 `send_input` 复用外部会话、超时取消外部回合并返回部分结果、父回合取消级联、未注册执行器与不可用客户端、执行器错误类型、`fork_context` 拒绝、模型覆盖与 Swarm 拒绝（external_agent_turn 11 passed，task 88 passed）
- 后台加 `AgentWait`：后台路径进入同一个 `execute_hidden_subagent_internal`，未单独测试——`start_background_subagent` 依赖进程级全局协调器，测试中无法隔离构造

## Task 2: 取消级联与权限归属

**Outcome:** 父回合取消和单独取消都能真正停止外部会话；权限请求带父会话归属，进入权限邮箱。

**Files:**
- Modify: `src/crates/interfaces/acp/src/client/manager.rs`
- Modify: `src/apps/desktop/src/api/agentic_api.rs`
- Test: `src/crates/assembly/core/src/agentic/coordination/external_agent_turn/tests.rs`
- Test: `src/crates/interfaces/acp/src/client/manager.rs`
- Test: `src/apps/desktop/src/api/agentic_api.rs`

**Change Map:**
- 取消级联由 Task 1 的子会话取消令牌承担：父回合取消、后台 `cancel`、超时都会取消该令牌，回合随即调用执行器取消，`coordinator.rs` 无需另接
- `manager.rs`：记录 ACP 会话所属的 OpenBitFun 会话；权限请求新增可选字段 `openbitfunSessionId`（`sessionId` 仍为 ACP 会话标识，旧前端不受影响）；父会话归属由前端经子会话的 `parentSessionId` 解析（Task 4）
- `agentic_api.rs`：`cancel_dialog_turn` 先取消 ACP 回合，再取消内核回合；仅当 ACP 已命中且内核无此回合（独立 ACP 会话）时忽略内核取消失败

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::coordination::external_agent_turn`
- `cargo test -p openbitfun-acp --no-default-features --features client,openbitfun-core/git --lib client::`
- `cargo test --locked -p openbitfun-desktop --lib dialog_turn_cancellation_settles`

- [x] 父回合取消级联到外部回合、超时取消外部回合的测试通过；桌面端取消结算测试覆盖委派子会话、独立 ACP 会话、原生会话三种情况；权限请求载荷带归属会话、未知时省略（acp `client::` 91 passed，桌面 1 passed）
- 权限进入远程控制与 Peer 的权限邮箱属于 F001-04

## Task 3: 旧工具退出模型视野与历史兼容

**Outcome:** 新会话的模型不再看到 `acp__…__prompt`，旧会话中的调用正常回放。

**Files:**
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/support.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/query.rs`
- Test: `src/crates/assembly/core/src/agentic/agents/registry/tests.rs`
- Test: `src/crates/interfaces/acp/src/client/tool.rs`

**Change Map:**
- `support.rs`、`query.rs`：模式工具的动态合并只合并 MCP，不再自动加入 `acp__` 工具
- 兼容：`acp__<client>__prompt` 仍注册在工具注册表中，显式列出它的自定义智能体与模式配置继续可用，历史工具卡片照常渲染；`manager.rs` 无需修改

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::agents::registry`
- `cargo test -p openbitfun-acp --no-default-features --features client,openbitfun-core/git --lib client::tool`

- [x] 模式合并测试证明 `acp__` 工具不再进入模式工具；旧 `acp__…__prompt` 结果载荷的渲染测试通过（registry 54 passed，tool 20 passed）

## Task 4: Web UI 子会话入口与状态

**Outcome:** 父会话的 `Task` 卡片可以打开 ACP 子会话，并显示后台运行中、等待审批、已取消、已完成几种状态。

**Files:**
- Modify: `src/web-ui/src/flow_chat/tool-cards/TaskToolDisplay.tsx`
- Modify: `src/web-ui/src/flow_chat/utils/subagentProjection.ts`
- Modify: `src/web-ui/src/flow_chat/services/flow-chat-manager/AcpPermissionToolCardModule.ts`
- Modify: `src/web-ui/src/infrastructure/api/service-api/ACPClientAPI.ts`
- Modify: `src/web-ui/src/locales/*/flow-chat.json`
- Test: `src/web-ui/src/flow_chat/tool-cards/TaskToolDisplay.test.tsx`
- Test: `src/web-ui/src/flow_chat/utils/subagentProjection.test.ts`
- Test: `src/web-ui/src/flow_chat/services/flow-chat-manager/AcpPermissionToolCardModule.test.ts`

**Change Map:**
- `TaskToolDisplay.tsx`：`acp:` 类型显示为「<client_id>（外部智能体）」，模型显示为「由外部智能体选择」；子会话有待审批工具时卡片为等待确认
- `subagentProjection.ts`：新增「运行中且等待审批」的判定，原生子智能体同样适用
- `AcpPermissionToolCardModule.ts`：子会话收到 ACP 权限请求时，父会话也标记需要处理
- 偏差：`FlowChatManager.ts` 无需修改——卡片已通过 `openBtwSessionInAuxPane` 按子会话标识打开

**Verification:**
- `pnpm --dir src/web-ui run test:run src/flow_chat/tool-cards/TaskToolDisplay.test.tsx src/flow_chat/utils/subagentProjection.test.ts src/flow_chat/services/flow-chat-manager/AcpPermissionToolCardModule.test.ts`
- `pnpm run type-check:web`
- `pnpm run i18n:audit`

- [x] 卡片测试覆盖运行中、等待确认、已取消、已完成四种状态与打开入口；投影与父会话提醒测试通过（48 passed）；类型检查与 i18n 审计通过

**Evidence notes:**
- Rust 验证以 `cargo +1.95.0` 执行（见 F001-01）。类型检查前需 `pnpm --dir src/web-ui run gen:types` 生成被忽略的 `@/generated/api`。
- `cargo check -p openbitfun-desktop`、`cargo check -p openbitfun-cli`、`pnpm run check:core-boundaries` 通过。
- `agentic::coordination` 共 246 passed、1 failed：`remote_session_metadata_never_falls_back_to_a_local_workspace_binding` 在本计划之前的 `HEAD`（`dc889165c`）上同样失败，与本计划无关。
- 已知限制：
  - 委派回合不向内核写入工具消息（`new_messages` 为空），持久化记录最终回答，工具轮次依赖前端检查点。
  - 外部智能体的上下文用量事件不展示。
  - 用户直接在 ACP 子会话面板输入时，走独立 ACP 会话路径（复用同一外部会话），不经内核。
  - `<available_agents>` 目录中的外部智能体条目在 F001-03 加入。
- 远程场景：仅本地测试，远程工作区、远程控制与 Peer 的验证在 F001-04。

## References
- `docs/superplan/rfcs/F001.md`
- `docs/superplan/plans/features/F001/01-external-agent-executor.md`
- `src/crates/assembly/core/src/agentic/execution/AGENTS.md`
- `src/web-ui/AGENTS.md`
