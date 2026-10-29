---
id: "F001-02"
title: "通过 Task 后台委派外部智能体"
type: "feature"
status: "draft"
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
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/validation.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/execution.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/schema.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/scheduler.rs`
- Modify: `src/crates/assembly/core/src/agentic/execution/execution_engine.rs`
- Test: `src/crates/assembly/core/src/agentic/tools/implementations/task/tests.rs`

**Change Map:**
- `validation.rs`：校验 `acp:` 类型的可用性，拒绝与 `fork_context` 同时使用
- `execution.rs`、`scheduler.rs`、`execution_engine.rs`：ACP 子会话的回合交给执行器，复用后台注册与结果交付
- `schema.rs`：在子智能体类型说明中列出可用的外部智能体

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::tools::implementations::task`

- [ ] 使用假执行器的测试覆盖：前台、后台加 `AgentWait`、`send_input`、超时、不可用、`fork_context` 拒绝

## Task 2: 取消级联与权限归属

**Outcome:** 父回合取消和单独取消都能真正停止外部会话；权限请求带父会话归属，进入权限邮箱。

**Files:**
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordinator.rs`
- Modify: `src/crates/interfaces/acp/src/client/manager.rs`
- Modify: `src/apps/desktop/src/api/agentic_api.rs`
- Test: `src/crates/assembly/core/src/agentic/tools/implementations/task/tests.rs`
- Test: `src/crates/interfaces/acp/src/client/executor.rs`

**Change Map:**
- `coordinator.rs`：后台 ACP 子会话的取消目标接到执行器取消
- `manager.rs`：权限请求带上子会话与父会话标识
- `agentic_api.rs`：`cancel_dialog_turn` 不再因为命中 ACP 会话就跳过父回合取消

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::tools::implementations::task`
- `cargo test -p openbitfun-acp --no-default-features --features client,openbitfun-core/git --lib client::executor`
- `cargo test --locked -p openbitfun-desktop --lib remote_workspace_policy`

- [ ] 测试证明父回合取消会级联到 ACP 子会话；单独取消只影响该子会话；权限请求带正确的父会话归属

## Task 3: 旧工具退出模型视野与历史兼容

**Outcome:** 新会话的模型不再看到 `acp__…__prompt`，旧会话中的调用正常回放。

**Files:**
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/support.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/query.rs`
- Modify: `src/crates/interfaces/acp/src/client/manager.rs`
- Test: `src/crates/assembly/core/src/agentic/agents/registry/tests.rs`

**Change Map:**
- `support.rs`、`query.rs`：动态合并不再把 `acp__` 工具交给模型
- `manager.rs`：保留工具的渲染与结果解析，供历史回放使用

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::agents::registry`

- [ ] 测试证明模式工具中已没有 `acp__` 工具；用旧载荷回放的测试通过

## Task 4: Web UI 子会话入口与状态

**Outcome:** 父会话的 `Task` 卡片可以打开 ACP 子会话，并显示后台运行中、等待审批、已取消、已完成几种状态。

**Files:**
- Modify: `src/web-ui/src/flow_chat/tool-cards/TaskToolDisplay.tsx`
- Modify: `src/web-ui/src/flow_chat/services/FlowChatManager.ts`
- Modify: `src/web-ui/src/locales/*/flow-chat.json`
- Test: `src/web-ui/src/flow_chat/tool-cards/TaskToolDisplay.test.tsx`

**Change Map:**
- `TaskToolDisplay.tsx`：识别 `acp:` 类型的子会话，显示外部智能体名称与状态，提供打开子会话的入口
- `FlowChatManager.ts`：按子会话标识打开 ACP 子会话

**Verification:**
- `pnpm --dir src/web-ui run test:run src/flow_chat/tool-cards/TaskToolDisplay.test.tsx`
- `pnpm run type-check:web`
- `pnpm run i18n:audit`

- [ ] 卡片测试覆盖四种状态与打开入口；类型检查与 i18n 审计通过

## References
- `docs/superplan/rfcs/F001.md`
- `docs/superplan/plans/features/F001/01-external-agent-executor.md`
- `src/crates/assembly/core/src/agentic/execution/AGENTS.md`
- `src/web-ui/AGENTS.md`
