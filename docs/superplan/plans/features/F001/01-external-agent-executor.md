---
id: "F001-01"
title: "宿主中立的外部智能体执行器（ACP）"
type: "feature"
status: "complete"
summary: "把 ACP 回合执行从桌面宿主命令下沉为内核可调度、宿主中立的外部智能体执行器，独立 ACP 会话行为保持不变。"
source: "docs/superplan/human/features.md"
created: "2026-10-29"
order: 1
depends_on: []
parent: "F001"
---

# 宿主中立的外部智能体执行器（ACP）Plan

**Goal:** 让内核能在任意宿主（桌面、CLI）中运行一轮 ACP 外部智能体回合，并通过共享事件发射器输出事件，为 `Task` 委派（F001-02）和远程场景（F001-04、F002）提供统一执行入口。

**Scope:**
- 在 `openbitfun-runtime-ports` 中新增外部智能体执行器端口：开始或恢复绑定到 OpenBitFun 会话的外部会话、执行一轮并流式返回事件、追加输入、取消、关闭，以及可用性探测。
- 在 `openbitfun-acp` 的 `client` 角色中实现该端口，复用现有的 `AcpClientService`、会话复用、远程连接参数、权限模式和取消通知。
- 把桌面端 `acp_client_api` 里「ACP 流事件 → `agentic://` 事件」的投影迁移到宿主中立层，经 `openbitfun_events::EventEmitter` 发出。桌面端命令只负责组装请求和交付事件。
- 桌面端与 CLI 在启动时把 ACP 执行器注册给内核；未注册时，内核报告「外部智能体不可用」。

**Non-Goals:**
- 不改变 `Task`、子智能体或智能体列表（F001-02、F001-03）。
- 不新增 Claude Agent SDK 实现。
- 不在 Detached Dispatch 工作进程中启用执行器（见 F002）。
- 不改变 ACP 配置格式与独立会话的入口交互。

**Architecture:**
- 依赖方向：端口在 Contracts 层（`runtime-ports`）；内核（`assembly/core`）只依赖端口；实现在 Interfaces 层（`interfaces/acp`）；宿主负责装配注册。`openbitfun-core` 不能依赖 `openbitfun-acp`。
- 事件投影是宿主中立的纯转换：输入 ACP 流事件，输出共享事件形态，不接触 `tauri::AppHandle`。
- 端口 DTO 只承载执行所需的事实：客户端标识、OpenBitFun 会话标识、工作区路径、远程连接标识、超时。ACP 协议细节留在 `openbitfun-acp`。
- 执行器必须能在 CLI 交付形态的无界面进程中构造，为 F002 第 2 阶段做准备。

**Baseline:**
- `src/apps/desktop/src/api/acp_client_api.rs:435` 在桌面命令里调用 `prompt_agent_stream`，逐条转成 `agentic://model-round-started` 等事件后直接 `app_handle.emit`。
- `AcpClientService` 只在桌面端（`app_state.rs`）和 CLI 的 `acp` 子命令（`acp_cli.rs`）中构造，CLI 智能体会话没有注册它。
- `cancel_openbitfun_session`（`manager.rs:1355`）按会话前缀发送 ACP 取消通知。

**Exit Criteria:**
- 独立 ACP 会话在桌面端的事件序列与迁移前一致，由等价测试证明。
- 内核可以通过端口为某个 OpenBitFun 会话执行一轮 ACP 回合，并经共享事件发射器收到流事件。在桌面与 CLI 宿主中均可构造。
- 未注册执行器时，内核得到明确的「不可用」结果，不会执行到一半失败。
- `pnpm run check:core-boundaries` 通过，没有新增越层依赖。

## Task 1: 定义外部智能体执行器端口

**Outcome:** `runtime-ports` 中有类型化的执行器端口和 DTO，并有基于假实现的端口行为测试。

**Files:**
- Create: `src/crates/contracts/runtime-ports/src/external_agent.rs`
- Modify: `src/crates/contracts/runtime-ports/src/lib.rs`
- Modify: `src/crates/contracts/runtime-ports/AGENTS.md`
- Modify: `src/crates/contracts/runtime-ports/Cargo.toml`
- Modify: `scripts/core-boundaries/rules/feature-rules.mjs`
- Modify: `scripts/core-boundaries/explicit-test-topology.mjs`
- Modify: `scripts/check-core-boundaries.test.mjs`
- Test: `src/crates/contracts/runtime-ports/tests/external_agent_port_contracts.rs`

**Change Map:**
- `external_agent.rs`：端口 trait、请求/事件/结果 DTO、可用性事实、取消入口
- `AGENTS.md`：在 Guardrails 中写明这个端口的职责范围
- 边界规则：新增 `external-agent-port`（依赖 `openbitfun-events`）、由 Core `agent-runtime` 转发、新增契约测试目标，形式与 `script-tool-runtime` 一致

**Verification:**
- `cargo test --locked -p openbitfun-runtime-ports --no-default-features --features external-agent-port --test external_agent_port_contracts`
- `pnpm run check:core-boundaries:test`

- [x] 端口和 DTO 已定义，假实现测试覆盖执行、追加（同会话的后续回合复用外部会话）、取消、不可用四种结果，以及跨版本容忍未知字段与缺失可选字段（5 passed）；边界规则自测 156 passed

## Task 2: 宿主中立的 ACP 事件投影与等价测试

**Outcome:** ACP 流事件到共享事件的投影移到 `openbitfun-acp`，有覆盖完整事件序列的等价测试。

**Files:**
- Create: `src/crates/interfaces/acp/src/client/event_projection.rs`
- Modify: `src/crates/interfaces/acp/src/client/mod.rs`
- Modify: `src/apps/desktop/src/api/acp_client_api.rs`
- Test: `src/crates/interfaces/acp/src/client/event_projection.rs`

**Change Map:**
- `event_projection.rs`：回合开始、模型轮次开始与结束、文本、思考、工具调用、权限请求、回合结束与错误的投影
- `acp_client_api.rs`：删除内联投影，改为调用共享投影并交付事件

**Verification:**
- `cargo test -p openbitfun-acp --no-default-features --features client,openbitfun-core/git --lib client::event_projection`
- `cargo check -p openbitfun-desktop`

- [x] 投影已迁移；等价测试的期望载荷逐字段取自迁移前桌面端的内联投影，覆盖轮次切换、思考、文本、工具、用量、命令、计划、配置、完成、取消、轮次前内容报错与回合开始/失败载荷

## Task 3: ACP 执行器实现与宿主注册

**Outcome:** `openbitfun-acp` 实现执行器端口；桌面与 CLI 启动时注册；内核可以经端口执行一轮回合。

**Files:**
- Create: `src/crates/interfaces/acp/src/client/executor.rs`
- Modify: `src/crates/interfaces/acp/src/client/mod.rs`
- Modify: `src/crates/assembly/core/Cargo.toml`
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordinator.rs`
- Modify: `src/apps/desktop/src/lib.rs`
- Modify: `src/apps/cli/src/acp_cli.rs`
- Modify: `src/apps/cli/src/agent/agentic_system.rs`
- Test: `src/crates/interfaces/acp/src/client/executor.rs`

**Change Map:**
- `executor.rs`：端口实现，委托给 `AcpClientService` 的会话解析、流式提示、取消；端口类型经 `openbitfun_core::runtime_ports` 引用，不新增 `acp` → `runtime-ports` 依赖边
- `coordinator.rs`：持有可选的执行器注册，提供读取入口；未注册时返回「不可用」（`ExecutorMissing`）
- `lib.rs`（桌面）：`AppState` 构造出 `AcpClientService` 后注册执行器
- `acp_cli.rs`、`agentic_system.rs`（CLI）：构造不注册旧版提示工具的 `AcpClientService`，并在智能体系统初始化时注册执行器；构造失败只记录警告

**Verification:**
- `cargo test -p openbitfun-acp --no-default-features --features client,openbitfun-core/git --lib client::executor`
- `cargo test -p openbitfun-acp --no-default-features --features client,openbitfun-core/git --lib client::prompt::tests`
- `cargo check -p openbitfun-desktop && cargo check -p openbitfun-cli`
- `pnpm run check:core-boundaries`

- [x] 执行器的可用性判定、流事件到端口事件与回合结果的映射、错误类型映射有单元测试；`client::` 共 90 passed（含既有 `client::prompt::tests` 的内存流测试）；桌面与 CLI 编译通过并完成注册；边界检查通过

**Evidence notes:**
- 本地默认工具链 Rust 1.91.1 无法编译 `main` 上既有的 `atomic_try_update` 用法（`1377cc6d2`）；CI 使用最新 stable，本计划的 Rust 验证均以 `cargo +1.95.0` 执行。
- 桌面编译前置：`pnpm install --frozen-lockfile`、`pnpm run plugin-host:prepare`、`pnpm run prepare:mobile-web`。
- 未覆盖、转入 F001-02：`AcpExternalAgentExecutor::run_turn` 经真实 `prompt_agent_stream` 的端到端测试（本计划仅编译验证该路径）。

## References
- `docs/superplan/rfcs/F001.md`
- `src/crates/interfaces/acp/AGENTS.md`
- `src/crates/contracts/runtime-ports/AGENTS.md`
- `docs/architecture/product-architecture.md`
- `docs/architecture/agent-runtime-deployment-design.md`
