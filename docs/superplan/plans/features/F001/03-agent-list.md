---
id: "F001-03"
title: "智能体列表展示外部智能体与可用状态"
type: "feature"
status: "complete"
summary: "启用的 ACP 外部智能体以执行方式字段出现在智能体列表中，显示可用状态与原因，详情链接到设置页。"
source: "docs/superplan/human/features.md"
created: "2026-10-29"
order: 3
depends_on: ["F001-01"]
parent: "F001"
---

# 智能体列表展示外部智能体与可用状态 Plan

**Goal:** 用户在智能体列表中能看到 Claude Code 等外部智能体，并知道它当前能否被委派、为什么不能。

**Scope:**
- 子智能体查询结果中加入外部智能体条目，新增一个可选的「执行方式」字段（例如 `acp`），带默认值；不向已有的来源枚举新增取值。
- 每个条目带可用状态：可用、未启用委派、未安装或探测失败、当前工作区不支持，并附原因。
- 外部智能体条目由 ACP 侧注册的目录提供者生成，内核只合并条目，不读取 ACP 配置。
- Web UI 智能体列表展示外部智能体、状态标记与原因；详情页提供跳转到「外部智能体」设置页的入口；列表中不提供启用开关。
- CLI peer host 的 `list_subagents` 返回同样的条目，与桌面端一致。

**Non-Goals:**
- 不在列表中编辑或启用外部智能体。
- 不展示远程设备上的智能体（F002）。
- 不改变 `Task` 的委派行为（F001-02）。

**Architecture:**
- 依赖方向与 F001-01 相同：内核依赖目录提供者接口，ACP 实现它，宿主负责注册。
- 跨版本：旧对端会忽略新的可选字段，旧版本的界面也不会因为未知的执行方式而解析失败；新界面在字段缺失时按原有子智能体处理。

**Baseline:**
- `list_subagents`（`src/apps/desktop/src/api/subagent_api.rs:96`）调用 `agent_registry.get_subagents_for_query`（`registry/query.rs:341`）。
- 来源枚举 `SubAgentSource` 是 `builtin | project | user | external`（`agent-runtime/src/agents.rs`），Web UI 对应 `SubagentSource`。
- CLI peer host 在 `peer_host/commands/tools.rs` 中处理 `list_subagents`。

**Exit Criteria:**
- 启用 Claude Code 后，智能体列表中出现 Claude Code，状态为「可用」；关闭它的委派后，状态变为「未启用委派」；未安装时显示探测失败的原因。
- 桌面端与 CLI peer host 返回的条目一致。
- 用旧载荷（不含执行方式字段）的测试证明旧数据仍能正常解析和显示。

## Task 1: 内核合并外部智能体目录条目

**Outcome:** 子智能体查询返回外部智能体条目，带执行方式与可用状态。

**Files:**
- Modify: `src/crates/contracts/runtime-ports/src/external_agent.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/query.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/types.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/external.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/mod.rs`
- Create: `src/crates/assembly/core/src/agentic/agents/registry/acp_agents.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/mod.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/execution.rs`
- Modify: `src/crates/interfaces/acp/src/client/executor.rs`
- Modify: `src/apps/desktop/src/api/subagent_api.rs`
- Test: `src/crates/contracts/runtime-ports/tests/external_agent_port_contracts.rs`
- Test: `src/crates/assembly/core/src/agentic/agents/registry/tests.rs`
- Test: `src/crates/assembly/core/src/agentic/tools/implementations/task/tests.rs`
- Test: `src/crates/interfaces/acp/src/client/executor.rs`

**Change Map:**
- `external_agent.rs`：新增 `ExternalAgentDescriptor`、`ExternalAgentListing { Configured, Probed }`，以及执行器端口上带默认实现的 `list_agents`
- `executor.rs`：可用性判定拆成「配置」与「探测」两步；`list_agents` 由 `list_clients()` 生成条目，`Probed` 时再做本机探测
- `types.rs`：`AgentInfo` 新增可选的 `executionKind`（`acp`）与 `externalAvailability`，均带默认值并在缺失时不序列化
- `acp_agents.rs`、`query.rs`：把条目投影为只读的 `acp:<client_id>` 子智能体，在管理列表（`RegistryManagement`）中合并，不覆盖已有 id
- `task/mod.rs`、`task/execution.rs`：`<available_agents>` 只在父会话能通过委派准入时列出可用的 ACP 条目；目录与准入共用 `external_agent_parent_refusal`
- `subagent_api.rs`：对 `acp:` 条目的修改请求返回明确的只读错误

**Deviations:**
- 没有新增独立的「目录提供者」接口，也没有 `directory.rs`：目录能力作为执行器端口的 `list_agents` 默认方法，沿用 F001-01 已有的唯一注册点。因此 `app_state.rs` 与 CLI `tools.rs` 无需改动；桌面端 `list_subagents` 与 CLI peer `list_subagents` 走同一个 `get_subagents_for_query`。
- `availability.rs` 未改动；ACP 条目的可用状态来自执行器，而不是本地覆盖层。
- 增加了 Task 目录的接入（原计划在 F001-02 只做准入）：目录使用 `Configured` 列表，避免每轮启动探测进程；未安装的客户端仍在委派准入时被拒绝并给出原因。
- 管理列表使用 `Probed` 列表，每次查询会探测本机客户端命令；远程工作区不列出 ACP 条目（`external_sources_supported = false`），远程场景见 F001-04。

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-runtime-ports --no-default-features --features external-agent-port --test external_agent_port_contracts`
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::agents::registry`
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::tools::implementations::task`
- `cargo +1.95.0 test -p openbitfun-acp --no-default-features --features client,openbitfun-core/git --lib client::`
- `cargo +1.95.0 test -p openbitfun-cli --bin openbitfun peer_host::`
- `cargo +1.95.0 test -p openbitfun-desktop --lib api::subagent_api`
- `pnpm run check:core-boundaries`

- [x] 端口契约 7 passed（含描述符容忍缺失字段与未知字段）；registry 57 passed，覆盖可用、未安装、未启用委派、未启用四种状态，不覆盖已有 id，以及不含新字段的旧载荷解析与往返；task 90 passed（目录与准入共用父会话拒绝条件）；acp `client::` 93 passed；CLI `peer_host::` 105 passed；desktop 1 passed；边界检查通过

**Evidence notes:**
- 「当前工作区不支持」（`unsupported_context`）在本地路径中不会产生；远程工作区目前直接不列出 ACP 条目，相关状态在 F001-04 处理。
- 桌面端与 CLI peer 一致性的依据是两者调用同一个查询；没有在注册了真实执行器的 CLI peer 进程上做端到端验证。

## Task 2: Web UI 列表与详情

**Outcome:** 智能体列表展示外部智能体、状态与原因，详情页可以跳转到设置页。

**Files:**
- Modify: `src/web-ui/src/infrastructure/api/service-api/SubagentAPI.ts`
- Modify: `src/web-ui/src/app/scenes/agents/agentsStore.ts`
- Modify: `src/web-ui/src/app/scenes/agents/agentVisibility.ts`
- Modify: `src/web-ui/src/app/scenes/agents/AgentsScene.tsx`
- Modify: `src/web-ui/src/locales/*/scenes/agents.json`
- Test: `src/web-ui/src/app/scenes/agents/AgentsScene.test.tsx`
- Test: `src/web-ui/src/app/scenes/agents/agentVisibility.test.ts`

**Change Map:**
- `SubagentAPI.ts`、`agentsStore.ts`：类型新增可选字段 `executionKind`、`externalAvailability`
- `agentVisibility.ts`：`isAcpExternalAgent`；ACP 条目沿用已有的 external 来源分组，因此不可在本地管理、没有启用开关
- `AgentsScene.tsx`：详情页显示可用状态标记（不可用时提示原因），提供跳转到「外部智能体」设置页（`tools.external-agents`）的入口；委派子智能体列表的提示中显示状态

**Deviations:**
- `useAgentsList.ts` 未改动：已有的 `external` 来源筛选与计数已覆盖 ACP 条目。

**Verification:**
- `pnpm --dir src/web-ui run test:run src/app/scenes/agents/AgentsScene.test.tsx src/app/scenes/agents/agentVisibility.test.ts`
- `pnpm run type-check:web`
- `pnpm run i18n:audit`

- [x] 2 个测试文件 22 passed（新增：ACP 详情显示「未安装」状态并跳转到外部智能体设置页；ACP 条目只读；缺少执行方式字段时按原有子智能体处理）；类型检查通过；i18n 审计 0 warning

**Evidence notes:**
- 只做了组件测试，没有在运行中的桌面应用里手动检查。所有验证都在本地工作区进行。

## References
- `docs/superplan/rfcs/F001.md`
- `docs/superplan/plans/features/F001/01-external-agent-executor.md`
- `src/web-ui/AGENTS.md`
- `src/apps/cli/AGENTS.md`
