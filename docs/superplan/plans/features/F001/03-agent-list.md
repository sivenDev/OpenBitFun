---
id: "F001-03"
title: "智能体列表展示外部智能体与可用状态"
type: "feature"
status: "approved"
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
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/query.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/types.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/availability.rs`
- Create: `src/crates/interfaces/acp/src/client/directory.rs`
- Modify: `src/apps/desktop/src/api/app_state.rs`
- Modify: `src/apps/cli/src/peer_host/commands/tools.rs`
- Test: `src/crates/assembly/core/src/agentic/agents/registry/tests.rs`
- Test: `src/crates/interfaces/acp/src/client/directory.rs`

**Change Map:**
- `types.rs`：子智能体信息新增可选的执行方式与可用状态字段
- `query.rs`、`availability.rs`：合并目录提供者的条目，计算可用状态
- `directory.rs`：根据 ACP 配置与探测结果生成条目
- `app_state.rs`、`tools.rs`：注册目录提供者，并在 CLI peer 路径中返回同样的结果

**Verification:**
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::agents::registry`
- `cargo test -p openbitfun-acp --no-default-features --features client,openbitfun-core/git --lib client::directory`
- `cargo test -p openbitfun-cli --bin openbitfun peer_host::`

- [ ] 测试覆盖四种可用状态和旧载荷兼容；桌面端与 CLI peer 返回一致

## Task 2: Web UI 列表与详情

**Outcome:** 智能体列表展示外部智能体、状态与原因，详情页可以跳转到设置页。

**Files:**
- Modify: `src/web-ui/src/infrastructure/api/service-api/SubagentAPI.ts`
- Modify: `src/web-ui/src/app/scenes/agents/hooks/useAgentsList.ts`
- Modify: `src/web-ui/src/app/scenes/agents/agentVisibility.ts`
- Modify: `src/web-ui/src/app/scenes/agents/AgentsScene.tsx`
- Modify: `src/web-ui/src/locales/*/scenes/agents.json`
- Test: `src/web-ui/src/app/scenes/agents/AgentsScene.test.tsx`
- Test: `src/web-ui/src/app/scenes/agents/agentVisibility.test.ts`

**Change Map:**
- `SubagentAPI.ts`：类型新增可选字段
- `useAgentsList.ts`、`agentVisibility.ts`：外部智能体条目的分组与可见性
- `AgentsScene.tsx`：状态标记、原因、跳转到设置页

**Verification:**
- `pnpm --dir src/web-ui run test:run src/app/scenes/agents/AgentsScene.test.tsx src/app/scenes/agents/agentVisibility.test.ts`
- `pnpm run type-check:web`
- `pnpm run i18n:audit`

- [ ] 场景测试覆盖可用、不可用及原因、跳转设置；类型检查与 i18n 审计通过

## References
- `docs/superplan/rfcs/F001.md`
- `docs/superplan/plans/features/F001/01-external-agent-executor.md`
- `src/web-ui/AGENTS.md`
- `src/apps/cli/AGENTS.md`
