---
id: "F002-02"
title: "同账号委派给远程原生智能体"
type: "feature"
status: "in_progress"
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

- [x] 远程条目在列表与目录中按可用性和准入规则出现

**Deviations:**
- 目录查询与缓存放在 core `service/dispatch/remote_agents.rs`（`RemoteAgentDirectory`，需要 `agent-runtime` 与 `ssh-remote`），而不是桌面端；桌面 `remote_agent_delegation.rs` 只提供账号设备列表与 relay RPC。原因：这段逻辑与平台无关，按仓库规则下沉。
- 目标端目录条目新增 `agentType`：F002-01 的准入要求请求中的智能体与开放项一致，发起方需要知道它。F002-01 尚未发布，没有旧数据。
- `remote-agent-port` 直接依赖 `product-domains`（待审批请求与回答类型），并在 core-boundaries 规则中登记为 `agent-runtime` 的转发项与桌面的直接消费项。
- 禁止多跳：目标端写入的 `dispatchDelegation` 原本只进入回合元数据，不进入工具上下文。现在协调器把它映射为上下文变量，工具上下文与子智能体都会继承；键名常量移到 core `service::delegation`，CLI 复用。
- 管理列表与每回合目录都使用 30 秒缓存，单台设备查询上限 8 秒；离线或不可达时保留上次的条目并标明原因。
- 提交、观察、取消、审批与继续在 Task 2 之前明确返回「本版本暂不支持」，不在本机执行替代。

**Evidence (2026-10-30):**
- `cargo +1.95.0 test --locked -p openbitfun-runtime-ports --features remote-agent-port --test remote_agent_port_contracts`：5 passed（寻址往返与非法地址、线上名称、目标状态载荷与未知状态、游标观察、离线与取消）。
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::agents::registry`：61 passed（含 `remote_agents` 投影、管理列表保留不可用条目、不遮盖已有 id）。
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib skill_agent`：10 passed（远程条目按准入出现；DeepReview 与被委派回合中不出现）。
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib tools::implementations::task`：94 passed（准入：未注册端口、未开放、离线、前台运行、指定模型、权限受限、DeepReview、被委派回合；子智能体继承禁止多跳标记）。
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib service::dispatch::remote_agents`：3 passed（只查询声明 `agent_delegation_v1` 的设备、缓存复用与离线原因、不可达设备）。
- `cargo +1.95.0 test --locked -p openbitfun-cli --bin openbitfun dispatch::`：103 passed；`cargo +1.95.0 check -p openbitfun-desktop`：通过；`pnpm run check:core-boundaries` 与 `node --test scripts/check-core-boundaries.test.mjs`（156）：通过。
- 远程场景：只有单元测试，尚未在真实设备或本机模拟中查询目录（见 Task 5）。

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

- [x] 后台委派、结果交付、取消、审批与重启补收都有内核测试覆盖

**Deviations:**
- 委派作业不写入发起端的 `OutboundDispatchStore`，作业关联以 JSON 存在协调库后台任务的新列 `remote_job`（设备、作业、会话与游标）。作业 id 使用前缀 `delegation-`，Dispatch 面板的目标作业接管会跳过它们，避免同一作业被两个观察器读取。
- `remote_job` 是可空列，在 `ensure_v2_additive_columns` 中原地补齐，不提升 schema 版本：旧版本仍能打开数据库并忽略该列；已有记录读出为无关联。
- `device_controller.rs` 未修改：提交、观察、取消、审批与继续直接在 `service/dispatch/remote_agents.rs` 的端口实现中调用 `dispatch_target_*`；桌面 `remote_agent_delegation.rs` 未修改，`lib.rs` 在注册端口后调用 `resume_remote_background_tasks`。
- 目标端的每个审批都在本机重新提问（不带可保存资源，不套用本机策略或自动批准），回答后再发回目标；目标端已处理的请求在本机撤回。
- 重启补收只继续观察并交付终态，不把重启前错过的过程事件重新投影到子会话。
- 设备暂时不可达时按 1.5 秒起、最长 30 秒退避重试，不把任务判为失败；取消时异步通知目标端取消。
- 编码任务在 Task 3 前由端口明确拒绝（不可用），不在本机替代执行。

**Evidence (2026-10-30):**
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib remote_agent`：13 passed（含 `remote_agent_turn`：最终文本只取作业会话最后一轮且跳过嵌套会话与未知事件；作业关联往返与无游标旧值、无法解析的值；按游标观察并重试不可达设备；失败原因来自目标端；取消观察会取消目标作业；目标审批在本机提问、回答发回目标、目标端已处理的请求在本机撤回）。
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib coordination_store`：18 passed（含关联、后续回合继续最近作业、本进程任务不算孤儿、重启后只接管一次且本地任务不被接管；无 `remote_job` 列的旧库补列后旧记录读出为无关联）。
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib service::dispatch`：57 passed；其中 `remote_agents` 5 个（提交载荷指明开放项且不带本机路径、审批与取消的线上形状、未开放与编码任务在发送前被拒绝）。
- `--lib tools::implementations::task`：94 passed；`--lib external_agent_turn`：23 passed；`--lib background_outcomes`：1 passed；`cargo +1.95.0 check -p openbitfun-desktop`：通过；`pnpm run check:core-boundaries`：通过。
- `--lib coordination` 中 `coordinator::tests::remote_session_metadata_never_falls_back_to_a_local_workspace_binding` 失败；在未改动的 HEAD（e9e7e95cb）干净工作树上同样失败，与本任务无关。
- 远程场景：只有内核测试，尚未在真实设备或本机模拟中跑通委派（见 Task 5）。

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

- [x] 编码任务的基线、同步与失败（非 Git 工作区、基线分叉）都有测试

**Deviations:**
- 编码任务复用设备 Dispatch 的整条基线流程（`submit_device_job`，即 `submit_device` 加 `delegation` 字段；结果用 `sync_device_result`），因此会写出站记录，由它持有基线 worktree 的保留声明；纯消息任务仍不写。出站记录列表（Dispatch 面板）不显示 `delegation-` 作业，避免第二个观察器。
- 基线只取发起工作区已提交的 `HEAD`（`include_uncommitted = false`），不带未提交改动；`task_kind` 的说明中告知模型。
- `AgentSpawn` 与 `Task` 新增可选参数 `task_kind`（`message` 默认 / `coding`），只对 `remote:` 类型有效，其他类型、`send_input`、`cancel` 与 DeepReview 都拒绝它；后续回合沿用作业记录中的任务类型（`RemoteJobLink.taskKind`，旧值读为 `message`）。
- 端口新增 `sync_coding_result`，默认实现返回不可用；桌面宿主在有应用路径时注册出站存储后才支持编码任务。
- 提交前拒绝：开放项不接受该任务类型、发起会话无工作区或是远程工作区、不是 Git 工作区；这些情况都不联系目标设备。
- 同步失败（如基线 worktree 自己有了提交，快进被拒）不判任务失败：回答照常交付，并附上无法同步的原因与作业 id，作业成果仍在目标设备上。每个成功回合（含后续回合与重启补收）都同步一次。
- `remote_agent_delegation.rs` 改为在注册端口时附上 `OutboundDispatchStore`；`baseline.rs` 只新增测试。

**Evidence (2026-10-30):**
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib remote_agent`：17 passed（新增：编码结果给出分支、提交范围、worktree 与改动文件；消息任务不同步；同步失败时保留回答并给出原因；无改动时如实说明；作业关联带任务类型且旧值读为 message）。
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib service::dispatch`：64 passed（新增：结果包快进基线分支；基线自有提交时拒绝且不改动；无出站存储、无工作区、非 Git 工作区在联系设备前被拒；同步回答映射为编码结果且以同步后的 head 为准；委派作业不进入 Dispatch 面板列表）。
- `--lib tools::implementations::task`：96 passed（`task_kind` 只用于生成 `remote:` 智能体、非法值与 `send_input` 被拒、AgentSpawn 透传与 schema；开放项不接受编码、无工作区、远程工作区被拒）。
- `cargo +1.95.0 test --locked -p openbitfun-runtime-ports --features remote-agent-port --test remote_agent_port_contracts`：7 passed；`cargo +1.95.0 check -p openbitfun-desktop` 与 `-p openbitfun-cli`：通过；`pnpm run check:core-boundaries`：通过。
- 远程场景：只有单元测试与本机 Git 仓库测试；编码任务端到端（真实目标设备的检出、提交与回传）留给 Task 4 实测与 Task 5 模拟。

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

**Progress (2026-10-30):** 界面部分完成，两台设备实测未做。
- 智能体详情显示远程条目所在设备、可用性（离线、需要更新、无响应，悬停显示原因）与可接受的任务类型；远程条目不提供本机管理入口，提示到开放它的设备上修改或关闭。
- Task 卡片把 `remote:<device>/<id>` 显示为「<id>（远程智能体）」，模型显示「由远程设备选择」。
- 转发到本机的目标端审批在审批栏写明「此操作在 <设备> 上执行，你的回答会发送到该设备」；为此转发时在显示元数据中加入 `remoteDeviceName`（来自缓存目录，取不到时用设备 id）。
- 偏差：「等待目标设备审批」不另设状态，目标端的审批以本机审批请求出现在子会话与父会话的审批栏中，由本机回答后送达目标端。
- Evidence：`pnpm --dir src/web-ui run test:run src/flow_chat/tool-cards/TaskToolDisplay.test.tsx src/flow_chat/components/ChatInputApprovalBand.test.tsx src/flow_chat/utils/remoteAgent.test.ts src/app/scenes/agents/AgentsScene.test.tsx src/app/scenes/agents/agentVisibility.test.ts`：79 passed；`pnpm run type-check:web`、`pnpm run i18n:audit`、`pnpm run check:web`：通过；`--lib remote_agent_turn`：9 passed（含 `remoteDeviceName`）。
- 远程场景：两台同账号设备的实测尚未进行；Task 5 的本机模拟先覆盖目录、委派、取消与审批。
- 模拟实测（2026-10-30，见 Task 5）：在真实桌面实例 A1（真实模型）中通过界面完成纯消息委派、审批（审批栏显示「此操作在 A4 Workstation 上执行」，允许后命令在 A4 执行）与取消（停止父回合后 A3 作业进入 `cancelled`）；智能体列表只列出同账号三台设备。实测发现两处界面问题并已修复：委派完成后 AgentSpawn 卡片一直显示「运行中」（见 Task 5 偏差）；审批栏标题显示原始 `remote:<设备>/<id>`，现显示「helper（远程智能体）」。

## Task 5: 本机双账号多客户端模拟

**Outcome:** 不依赖真实设备，在一台电脑上模拟两个账号、多个客户端，自动化覆盖委派流程，并提供一条启动命令供人工体验。经用户批准加入本计划（2026-10-30）。

**Scope:**
- **自动化模拟**：
  - 不改动产品代码。测试在 127.0.0.1 上进程内启动 relay-service，使用临时数据库，直接写入用户、设备与令牌。
  - 被模拟的设备是真实的 `openbitfun` CLI 进程，各自使用隔离的 `OPENBITFUN_HOME`、`OPENBITFUN_USER_ROOT` 与 `OPENBITFUN_DEVICE_IDENTITY_PATH`。
  - 不使用 `daemon __dispatch_provision`（会安装 launchd 服务）；直接写入会话，不安装任何服务，结束后删除临时目录。
- **开发脚本**：
  - 新增 `scripts/dev/sim-devices.mjs`：启动本地 relay 与 CLI 设备（如 A2、B1），再启动一个使用隔离存储、指向本地 relay 的桌面实例（A1）。
  - 桌面端的 relay 覆盖只在调试或开发构建中生效，发布构建中不存在。
- **模拟不能证明**：真实 GitHub 登录、真实网络、新旧版本混用、手机端与 IM。

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-cli --test cli_command_contracts delegation_simulation::`
- `RUSTUP_TOOLCHAIN=1.95.0 node scripts/dev/sim-devices.mjs --check`
- 手动：运行开发脚本，在桌面实例中完成一次纯消息委派
- `pnpm run check:repo-hygiene`

- [x] 自动化模拟覆盖目录、委派、取消与审批，开发脚本可一条命令启动

**Deviations:**
- 自动化模拟放在已有的 `cli_command_contracts` 测试目标中（`delegation_simulation`），不新增 `[[test]]`。它直接走委派作业的线上协议；被模拟的设备为 A2（立即回答）、A3（挂起直到取消）、A4（执行需审批的命令）与另一账号的 B1。
- 桌面端的请求方检查是一个被忽略的桌面库测试（`remote_agent_delegation::simulation_tests`），由 `scripts/dev/sim-devices.mjs --check` 读取模拟清单后运行；它复用桌面的 HostInvoke 封装（`dispatch_api.rs` 中提取的 `invoke_device_dispatch`），只替换登录会话。
- 不需要新增桌面端 relay 覆盖：恢复的会话使用其持久化的 relay 地址，桌面已有的 E2E 隔离（`OPENBITFUN_USER_ROOT`/`OPENBITFUN_HOME` 与存储保护）隔离配置、会话、设备身份与 WebView，并跳过单实例。因此没有发布构建中的覆盖代码。
- `--copy-models` 只从开发者自己的配置复制 `ai` 一节（含 API 密钥）到权限为 700 的临时目录，退出时删除；机器人、relay 与工作区设置不会进入模拟。
- 产品修复一：`daemon run` 这类长寿命父进程不回收分离的 Dispatch 工作进程，僵尸进程组仍响应 `kill(0)`，导致取消委派作业报错。`runner.rs` 现在用回收线程等待子进程。去掉修复时模拟测试的取消步骤失败。
- 产品修复二：外部（`acp:`）与远程（`remote:`）子智能体回合不经过执行引擎，子会话从未收到 `DialogTurnCompleted`，界面卡片一直显示「运行中」。协调器现在在这类回合成功时补发该事件（失败与取消原本就由协调器发出）。该问题同样影响 F001 的外部智能体。
- 审批栏标题对远程智能体显示「<开放项>（远程智能体）」。
- 模拟中的设备使用模拟模型，回答是测试标记文本（如 `ACTIVE_TURN_STREAM_MARKER`），启动脚本会说明。

**Evidence (2026-10-30):**
- `cargo +1.95.0 test --locked -p openbitfun-cli --test cli_command_contracts delegation_simulation::`：1 passed（目录、另一账号不可达、纯消息、取消、审批后命令在目标执行），约 7–10 秒，连续三次稳定；结束后无残留守护进程与临时目录。`--test cli_command_contracts`：42 passed；`--bin openbitfun dispatch`：111 passed。
- `RUSTUP_TOOLCHAIN=1.95.0 node scripts/dev/sim-devices.mjs --check`：桌面请求方检查 1 passed（远程条目只含同账号三台设备、另一账号返回 NotFound、纯消息成功、取消后为 cancelled、审批回答后成功）。
- `node scripts/dev/sim-devices.mjs --copy-models`（经嵌入式 WebDriver 驱动界面）：A1 的 `list_subagents` 返回三个 `remote:` 条目及设备名与可用性；真实模型调用 `AgentSpawn(remote:…a02/helper)`，`AgentWait` 取回 A2 的回答；A4 的审批在 A1 审批栏出现并在允许后于 A4 执行；停止父回合 5 毫秒内 A3 作业记录 `cancelRequestedAt` 并进入 `cancelled`。Ctrl+C 后所有进程退出，临时目录（含复制的密钥）被删除。
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib external_agent_turn`：24 passed（新增子回合完成事件测试；去掉修复时失败）。`--lib -- remote_agent coordination_store tools::implementations::task background_outcomes agentic::coordination`：369 passed，1 failed（`remote_session_metadata_never_falls_back_to_a_local_workspace_binding`，在干净 HEAD 上同样失败，见 Task 2）。
- `pnpm --dir src/web-ui run test:run src/flow_chat/components/ChatInputApprovalBand.test.tsx`：14 passed；`pnpm run type-check:web`、`pnpm run i18n:audit`：通过；`pnpm run lint:web`：0 errors。
- `pnpm run check:core-boundaries`、`pnpm run check:repo-hygiene`：通过。
- 模拟不能证明：真实 GitHub 登录、真实网络、新旧版本混用、手机端与 IM。

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/01-target-open-catalog.md`
- `docs/superplan/plans/features/F001/02-task-delegation.md`
- `docs/superplan/plans/features/F001/03-agent-list.md`
- `docs/architecture/detached-task-dispatch.md`
- `docs/architecture/peer-device-mode.md`
