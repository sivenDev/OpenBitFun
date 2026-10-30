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
- Modify: `src/crates/services/services-integrations/src/remote_connect/`（`RemoteCommand` 与机器人命令路由中的委派状态、取消、权限）
- Modify: `src/mobile-web/src/`（后台委派状态展示）
- Modify: `src/apps/cli/src/peer_host/commands/mod.rs`
- Modify: `src/crates/contracts/product-domains/src/remote_surface/table.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/task/validation.rs`
- Test: `src/apps/cli/src/peer_host/commands/mod.rs`
- Test: `src/crates/contracts/product-domains/src/remote_surface/`

**Change Map:**
- `remote_connect/`：委派状态、取消、权限请求的远程命令与机器人回复
- `peer_host/commands/mod.rs`：处理新命令，并保持 `cli_peer: HANDLED` 闭包一致
- `table.rs`：新增或变更命令的行
- `validation.rs`：Dispatch 交付形态下拒绝 `acp:` 类型

**Verification:**
- `cargo test -p openbitfun-product-domains --no-default-features remote_surface`
- `cargo test -p openbitfun-cli --bin openbitfun peer_host::`
- `pnpm run capabilities:generate && pnpm run capabilities:check`
- `pnpm run type-check:mobile-web`
- `pnpm run check:core-boundaries`

- [ ] 注册表与闭包测试通过；手机端类型检查通过；Dispatch 拒绝路径有测试

## Task 3: 远程场景实测记录

**Outcome:** 每种远程场景都有实际运行的证据，记录在本计划中。

**Files:**
- Modify: `docs/superplan/plans/features/F001/04-remote-scenarios.md`

**Verification:**
- 手动：在 SSH 远程工作区、手机端远程控制、Peer 模式下各执行一次后台委派与取消，记录结果；在 Dispatch 任务中验证明确失败的提示

- [ ] 四种场景的实测结果已记录

## References
- `docs/superplan/rfcs/F001.md`
- `docs/superplan/plans/features/F001/02-task-delegation.md`
- `docs/superplan/plans/features/F001/03-agent-list.md`
- `docs/architecture/remote-surface-contract.md`
- `src/crates/services/services-integrations/AGENTS.md`
- `src/mobile-web/AGENTS.md`
