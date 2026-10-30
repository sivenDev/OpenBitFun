---
id: "F002-06"
title: "跨账号委派：按连接授权、目标端审批、额度与审计"
type: "feature"
status: "approved"
summary: "目标主人把开放项逐项授权给某个连接；发起方经连接看到并委派远程智能体；危险操作只由目标主人审批；按连接设置并发与时长额度；跨账号委派写入目标端审计；撤销后进行中的任务被取消。"
source: "docs/superplan/human/features.md"
created: "2026-10-30"
order: 6
depends_on: ["F002-03", "F002-05"]
parent: "F002"
---

# 跨账号委派：按连接授权、目标端审批、额度与审计 Plan

**Goal:** 不同账号之间可以安全地委派任务：目标主人决定开放什么、给谁、能用多少，并保有最终审批权，随时可以收回。

**Scope:**
- **按连接授权**：
  - 开放项的授权增加「连接」维度，跨账号必须逐项授权，默认不授权。
  - 任意 ACP 外部智能体（如 Claude Code、Codex）的跨账号授权默认关闭，授权前按智能体再次提示额度与条款风险（Claude Code 另有订阅个人使用的说明），并需要显式确认。
- **目录与列表**：发起方经有效连接查询对方目录，只返回授权给该连接的条目。智能体列表中的这些条目标明所属账号、设备、在线状态与兼容性。目标声明 `cross_account_delegation_v1` 时才查询。
- **委派**：
  - 沿用 F002-02 的寻址、后台运行、观察、结果回收与重启补收，传输换成 F002-05 的连接 RPC。
  - 寻址中包含连接 id，与同账号条目区分。
- **审批**：
  - 跨账号任务的审批请求只呈现给目标主人。
  - 发起方只能看到「等待对方审批」，来自连接的 `answer` 一律被拒绝。
  - 目标主人可以为开放项设置更宽松的策略（例如只读操作自动通过），但策略不能放宽到超出该开放项的权限上限。
- **额度**：每个开放项可按连接设置并发任务数与单任务时长上限。超出时提交被拒绝，超时的任务被取消，原因都明确返回发起方。
- **审计**：目标端记录每次跨账号委派的来源账号、设备、开放项、任务类型、开始与结束时间和结果状态，主人可以在设置页查看。
- **撤销**：撤销连接或授权时，该连接上进行中的任务被取消。目标端会话保留在目标主人本地；发起方收到「授权已撤销」的结果，新的委派被拒绝。

**Non-Goals:**
- 不做账号之间的计费结算。
- 不做公开智能体市场或按陌生人检索。
- 不做多跳委派。
- 不做 relay 侧排队。

**Architecture:**
- 目标端依据连接身份判定授权、审批权与额度：连接身份来自连接 RPC 的认证结果，不采信载荷中自报的账号或设备。
- 额度属于授权边界上的资源控制，在提交与调度处执行，不进入智能体循环。
- 审计是目标端新增的本地数据，只追加。读取宽容，损坏的条目保留并标记。
- 发起方复用 F002-02 的端口与远程委派路径，只增加连接作为目标类型。不新增第二套后台、等待或取消语义。
- 跨版本：目标端没有 `cross_account_delegation_v1` 时不展示也不委派；relay 没有 `cross_account_routing_v1` 时整体不可用（F002-05）。

**Baseline:**
- F002-01 提供开放清单、目录与带委派的提交；F002-02 提供发起方委派路径；F002-03 提供远程外部智能体；F002-05 提供连接与连接 RPC。
- Dispatch 的 `remote` 审批策略、`status` 中的待审批请求和 `answer` 动词已经可用。

**Exit Criteria:**
- 两个账号完成配对后，未逐项授权时发起方看不到任何条目；授权后可见，并标明对方账号与设备。
- 跨账号委派纯消息与编码任务时，发起方不被阻塞，结果回到发起会话。
- 目标端的危险操作只能由目标主人批准；发起方显示「等待对方审批」，从连接发来的 `answer` 被拒绝。
- 超过并发或时长额度时，委派明确失败或被取消，并给出原因。
- 撤销连接后，进行中的任务被取消，发起方收到「授权已撤销」，新的委派被拒绝。
- 目标端审计中可以看到每次跨账号委派。
- 外部智能体的跨账号授权需要显式确认，默认关闭。

## Task 1: 按连接授权与目录

**Outcome:** 开放项可以逐项授权给连接，目标端按连接身份过滤目录并校验提交，声明 `cross_account_delegation_v1`。

**Files:**
- Modify: `src/crates/assembly/core/src/service/delegation.rs`
- Modify: `src/apps/cli/src/dispatch/mod.rs`
- Modify: `src/crates/services/services-core/src/dispatch_contract.rs`
- Modify: `src/web-ui/src/features/dispatch/dispatch.contract.test.ts`
- Modify: `src/web-ui/src/infrastructure/config/components/AgentDelegationConfig.tsx`
- Test: `src/crates/assembly/core/src/service/delegation.rs`

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib service::delegation`
- `cargo +1.95.0 test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `pnpm --dir src/web-ui run test:run src/features/dispatch/dispatch.contract.test.ts`

- [ ] 授权、目录过滤与提交校验按连接身份执行，外部智能体需要显式确认

## Task 2: 发起方经连接委派

**Outcome:** 发起方列出连接上的远程智能体并委派，结果回收、取消与重启补收与同账号一致。

**Files:**
- Modify: `src/crates/contracts/runtime-ports/src/remote_agent.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/remote_agents.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/remote_agent_turn.rs`
- Modify: `src/apps/desktop/src/api/remote_agent_delegation.rs`
- Test: `src/crates/assembly/core/src/agentic/coordination/remote_agent_turn/tests.rs`

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib remote_agent`
- `cargo +1.95.0 check -p openbitfun-desktop`

- [ ] 连接目标的列表、委派与结果回收有测试，同账号路径回归通过

## Task 3: 目标端审批、额度、审计与撤销

**Outcome:** 跨账号任务只由目标主人审批，额度在提交与调度处生效，审计可查看，撤销取消进行中的任务。

**Files:**
- Modify: `src/apps/cli/src/dispatch/mod.rs`
- Modify: `src/apps/cli/src/dispatch/permissions.rs`
- Modify: `src/apps/cli/src/dispatch/runner.rs`
- Create: `src/apps/cli/src/dispatch/audit.rs`
- Create: `src/web-ui/src/infrastructure/config/components/DelegationAuditView.tsx`
- Modify: `src/web-ui/src/locales/*/` 相关文案
- Test: `src/apps/cli/src/dispatch/mod.rs`

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `pnpm --dir src/web-ui run test:run <changed test files>`
- `pnpm run i18n:audit`

- [ ] 审批权、额度、审计与撤销都有测试

## Task 4: 两个账号的端到端实测

**Outcome:** 两个账号的设备完成配对、授权、委派、审批、额度、撤销与审计的完整流程，结果记录在本计划中。

**Files:**
- Modify: `docs/superplan/plans/features/F002/06-cross-account-delegation.md`

**Verification:**
- 手动：按 RFC 批准条件 1 至 8 逐项实测，并记录结果

- [ ] RFC 批准条件逐项实测并记录

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/01-target-open-catalog.md`
- `docs/superplan/plans/features/F002/02-same-account-delegation.md`
- `docs/superplan/plans/features/F002/03-remote-external-agents.md`
- `docs/superplan/plans/features/F002/05-cross-account-pairing.md`
- `docs/architecture/detached-task-dispatch.md`
