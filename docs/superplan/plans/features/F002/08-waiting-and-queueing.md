---
id: "F002-08"
title: "AI协作：外部任务受阻时如实告知，超出额度时排队"
type: "feature"
status: "superseded"
summary: "跨账号任务等待对方主人审批或因并发额度排队时，发起方助理立刻得知原因并告诉用户、不重发；超出额度的任务在目标端排队、按先后开始，而不是被拒绝。"
source: "docs/superplan/human/features.md"
created: "2026-11-01"
order: 8
depends_on: ["F002-07"]
parent: "F002"
---

# AI协作：外部任务受阻时如实告知，超出额度时排队 Plan

> 已被 F002 RFC 第 6 版（局域网共享助理）取代，替代计划为 F002-10 至 F002-13（待 RFC 批准后制定）。本计划交付的成果中，Dispatch 执行与审批、外部任务以助理身份运行且不改写助理、受阻时如实告知与排队、会话保留期限，按 RFC D12 迁移进新流程，其余随第 6 版删除。

**需求基准：** RFC `docs/superplan/rfcs/F002.md` 的「AI协作需求（基准）」一节。本计划来自与 OpenBot 的设计对比（主人 2026-11-01 批准「按建议实现」第 1、2 项），只借鉴思路，不复制代码。

**Goal:** 发起方的助理在远程任务卡住时不再干等或重发，主人设置的并发额度不再让对方的任务直接失败。

**Scope:**
- **受阻时如实告知（发起方）**：
  - 跨账号任务等待目标主人审批、或在目标端排队时，发起方记录这个子智能体的受阻原因（哪台设备、在等什么、怎样解除）。
  - `AgentWait` 在没有可交付结果、且有尚未报告过的受阻原因时提前返回 `blocked`，结果中逐个列出受阻任务、原因与「Unblock:」一行，并说明任务仍在运行、之后再次 `AgentWait` 取结果、不要重发同一请求。
  - 同一次受阻只提前返回一次；再次 `AgentWait` 正常等待，直到出现新的受阻或任务结束。超时结果也带上仍然有效的受阻原因。
- **超出额度时排队（目标端）**：
  - 提交与后续回合不再因「同时任务数」被拒绝；任务被接受并保持 `queued`，工作进程等到该连接、该助理正在运行的任务少于额度时再开始，按开始等待的先后顺序。
  - 排队期间目标端记录排队原因（额度与开始等待的时间），`status` 返回可选的 `hold` 字段；取消、收回授权在排队期间同样生效。
  - 发起方子会话显示「在某设备上排队」，并作为上面的受阻原因报告给助理。

**Non-Goals:**
- 不在远程助理列表中显示忙闲状态（需要新的对端能力协商，留到后续）。
- 不合并一次派给多个助理的结果。
- 不改变同账号任务的审批方式（提示仍转发给发起方用户回答，不算受阻）。
- 不改主人一侧的界面。

**Architecture:**
- **兼容**：`hold` 是 `status` 的可选字段，旧发起方忽略它并照常观察 `queued` 状态；旧目标仍按旧规则拒绝，新发起方照常报错。两边都不依赖对方的新行为，不新增协商能力，协议版本不变。
- **排队**：沿用工作进程等待工作区锁时保持 `queued` 的做法。占用名额的是状态为 `running` 的任务；在连接准入锁内计数并标记 `running`，保证并发提交不超额。排队记录是任务目录中的一个文件，工作进程结束时清除。
- **受阻原因**：发起方进程内按子会话记录（不持久化）；重启后由恢复观察的路径重新得到。只给模型可见的英文说明，日志仍为英文、无表情符号。

**Baseline:**
- `connection_guard::ensure_within_limits` 在 `submit` 与 `continue_job` 中统计该连接、该开放项未结束的任务，达到上限即报错拒绝。
- 发起方在跨账号任务等待目标主人时只向子会话写一条说明（`owner_approval_note`）；`AgentWait` 至少等待 30 分钟，超时结果只列出未完成任务的编号，不说明原因。

**Exit Criteria:**
- 超出额度的第二个任务被接受、显示排队，第一个结束后自动开始并成功（CLI 与桌面两账号模拟）。
- 跨账号任务等待目标主人审批时，`AgentWait` 提前返回 `blocked` 并说明原因与解除方式；再次等待能取到最终结果。

## Task 1: 目标端超出额度时排队

**Outcome:** 超出「同时任务数」的任务与后续回合被接受并排队，名额空出后按先后开始；`status` 返回排队原因。

**Files:**
- Modify: `src/apps/cli/src/dispatch/{connection_guard,mod,worker,store,protocol}.rs`
- Test: `src/apps/cli/src/dispatch/connection_guard.rs`、`src/apps/cli/tests/cli_command_contracts/delegation_simulation.rs`

**Change Map:**
- `connection_guard.rs`: `ensure_within_limits` 改为工作进程使用的「等待名额」：准入锁内计数 `running` 任务与更早开始等待的任务，有名额即返回锁；否则写排队记录并定期重试，期间检查取消与授权。
- `worker.rs`: 标记 `running` 前等待名额；结束时清除排队记录。
- `mod.rs`: `submit`、`continue_job` 不再按额度拒绝；`status` 带上 `hold`。
- `store.rs`、`protocol.rs`: 排队记录的读写与 `hold` 字段。

**Verification:**
- `cargo test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `cargo test --locked -p openbitfun-cli --test cli_command_contracts delegation`

- [x] 超出额度的任务排队并按先后开始，取消与收回授权在排队期间生效，均有测试
  - 证据（2026-11-01）：
    - `connection_guard::wait_for_place` 取代提交时的拒绝；工作进程在标记 `running` 前等待名额，名额判断在连接准入锁内进行，结束时清除 `hold.json`；`status` 在 `queued` 时返回 `hold`。
    - CLI `dispatch::` 116 + 1 项通过，新增 `tasks_over_the_limit_wait_in_order_and_start_when_a_place_frees`（排队原因、先等先开始、不同连接分开计数）与 `a_waiting_task_stops_when_cancelled_or_its_grant_is_withdrawn`。
    - CLI 合约 `delegation` 4 项通过：`devices_of_different_accounts_pair_and_call_over_a_connection` 中第二个任务被接受并返回 `hold.reason = connectionLimit`、`limit = 1`，收回授权后两个任务都以「收回访问」失败，审计记录两者。

## Task 2: 发起方在任务受阻时如实告知

**Outcome:** 发起方助理在远程任务等待目标主人或排队时，从 `AgentWait` 立刻得知原因并告诉用户，不重发；之后仍能取到结果。

**Files:**
- Modify: `src/crates/contracts/runtime-ports/src/remote_agent.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/{remote_agent_turn.rs,background_outcomes.rs,mod.rs}`
- Create: `src/crates/assembly/core/src/agentic/coordination/remote_holds.rs`
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/agent_wait_tool.rs`
- Modify: `src/apps/desktop/src/api/remote_agent_delegation/simulation_tests.rs`
- Test: 上述模块测试

**Change Map:**
- `remote_agent.rs`: `RemoteAgentJobSnapshot.hold`（可选，未知原因可解析）。
- `remote_holds.rs`: 按子会话记录受阻原因与是否已报告，变化时唤醒等待。
- `remote_agent_turn.rs`: 等待目标主人或排队时设置受阻原因，恢复运行或结束时清除；排队时向子会话写一条说明。
- `background_outcomes.rs`、`agent_wait_tool.rs`: 新的等待结果 `blocked` 与受阻列表。

**Verification:**
- `cargo test --locked -p openbitfun-runtime-ports --features remote-agent-port`
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,dispatch-store,ssh-remote --lib -- remote_agent background_outcomes agent_wait`
- `cargo test -p openbitfun-desktop --lib remote_agent_delegation`（两账号模拟）

- [x] 受阻时 `AgentWait` 提前返回并说明原因，同一受阻只报告一次，均有测试
  - 证据（2026-11-01）：
    - runtime-ports 测试通过：旧目标不带 `hold`，新原因解析为 `Unknown`。
    - core（`remote_agent background_outcomes agent_wait remote_holds`）54 项通过：`the_parent_hears_why_a_job_waits_on_its_device`（排队与等待对方主人时记录原因、同账号提示不算受阻、结束清除）、`a_held_remote_agent_ends_one_wait_early_and_is_listed_after`（第一次等待返回 `blocked`，第二次正常超时但仍列出原因）、`assistant_result_says_why_a_task_is_blocked_and_not_to_resend`。
    - 有新受阻时，已就绪的结果与 `blocked` 一并交付。Web UI 的 AgentWait 卡片对 `blocked` 显示「N 个任务在其他设备上等待，仍在继续」，测试 17 项通过。
    - 两账号模拟 `node scripts/dev/sim-devices.mjs --check` 通过：B1 一次只运行一个任务，第二个任务排队并带 `hold`，第一个获批完成后自动开始并成功。

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/06-cross-account-delegation.md`
- `docs/superplan/plans/features/F002/07-open-assistants.md`
- `docs/architecture/detached-task-dispatch.md`
