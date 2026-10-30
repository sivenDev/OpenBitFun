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
- F002-01 只为 `catalog` 与带委派的 `submit` 绑定来源设备；`status`、`cancel`、`answer`、`append`、`continue`、`query` 目前不核对来源。跨账号时这些动词必须按任务记录中的来源（连接）校验，不属于该连接的任务一律拒绝。
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

- [x] 授权、目录过滤与提交校验按连接身份执行，外部智能体需要显式确认
  - 实现：
    - 开放项新增 `connections`（逐项授权，默认为空，旧条目照常读取）；只经 `grant_connection` / `revoke_connection` 管理，重新开放同一智能体保留授权，换成别的智能体时清空。外部智能体授权给连接需要再次确认使用提示。同账号设备授权与连接授权互不通用。
    - 连接请求由宿主绑定 `__sourceConnectionId`（只取自认证过的连接，并去掉两种自报来源）；连接只能发 probe、catalog、带委派的 submit，以及对自己任务的 status、cancel、query、list、continue、append。`answer` 一律拒绝；工作区相关动词先不开放，编码任务在 Task 2 中按连接记录工作区归属后再开放。
    - 目标端在动词执行前统一检查：任务必须由该连接提交（否则按不存在处理）；query 只允许用量报告，不能读文件；continue 不能改审批策略和模型。提交时清除发起方指定的模型与推理强度，由目标端决定。
    - 连接的 probe 只回答能力与是否可运行，不返回模型名单、目录与诊断，也不能检查目标端路径。
    - 目标端声明 `cross_account_delegation_v1`。CLI 守护进程与桌面端都把连接请求交给分发执行器，不再一律拒绝；桌面端调用的旧版 CLI 会因不认识该字段而明确报错。
    - 管理入口：CLI `delegation grant-connection` / `revoke-connection`，`delegation list` 显示授权；桌面命令 `delegation_grant_connection` / `delegation_revoke_connection`（远程操作登记为 ControllerLocal）；设置页可按开放项授权给可用连接、显示和收回授权，外部智能体需勾选确认，切换账号后需要重新确认。只能授权给本设备保存且未撤销的连接。
  - 证据：
    - 核心 `service::delegation` 10 项通过（含逐项授权、与设备授权分离、外部智能体确认、重新开放保留授权）；`connection_rpc` 12 项通过（含可授权连接检查）；`dispatch_contract` 8 项通过（含连接绑定与命令限制）；
    - CLI `dispatch::` 110 项通过，其中 `dispatch::connection` 覆盖任务归属、读文件与改审批/模型的拒绝、动词限制，目录与准入测试覆盖连接授权和独立的消息目录；
    - CLI 合约测试 `devices_of_different_accounts_pair_and_call_over_a_connection` 用真实守护进程验证：未授权时目录为空（即使对本账号全部设备开放）；probe 带新能力且不含模型名单，不能检查路径；`delegation grant-connection` 后目录可见，收回后为空；`answer`、他人任务、工作区动词均被拒绝。同账号委派的合约测试仍通过；
    - `AgentDelegationConfig.test.tsx` 9 项、`dispatch.contract.test.ts` 12 项通过；`type-check:web`、`i18n:audit`（0 警告）、`capabilities:check` 通过；`remote_surface` 20 项通过；桌面 `cargo check -p openbitfun-desktop --lib` 通过。

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

- [x] 连接目标的列表、委派与结果回收有测试，同账号路径回归通过
  - 实现：
    - 寻址：连接目标的 id 为 `conn:<connectionId>`，智能体类型为 `remote:conn:<connectionId>/<openAgentId>`，与同账号条目互不冲突；账号设备 id 不含 `:`。端口描述新增可选的 `account`（对方账号的用户 id 与显示名），旧数据没有该字段时照常读取。
    - 发起方目录 `RemoteAgentDirectory::with_connections`：列出本设备可用的连接（已确认、未撤销、对方密钥已核验），只在目标声明 `cross_account_delegation_v1` 时查询其目录；账号设备列表读取失败时连接条目照常列出。一个路由 RPC 把 `conn:` 目标的所有调用（提交、观察、取消、续问、编码基线上传与结果同步）经连接发送，其他调用仍走账号设备 RPC，后台运行、观察、取消与重启补收沿用同一套代码。
    - 桌面端用本设备自己的路由连接（Relay 只转发来自设备连接的跨账号调用）调用 `ConnectionRpc`；可用连接取自本地连接记录。
    - 发起会话：跨账号任务的审批请求不会进入本机审批信箱，子会话改为提示「正在等待对方设备的主人审批」。注册表条目与智能体详情标明对方账号与设备，模型看到的描述说明该智能体属于其他账号、由对方审批。
    - 目标端（为编码任务开放工作区动词）：
      - 工作区按任务记录提供它的连接，其他连接或本账号已开始的工作区一律按不存在处理；
      - 连接的仓库缓存按连接隔离（`repoKey` 按连接重新派生），且不接受 `remoteUrl`，代码只来自发起方上传的 bundle，目标端不会用主人的 Git 凭据拉取对方指定的地址；
      - 编码任务只能在为同一任务、同一连接准备的 worktree 中运行。
    - 目标端其他收紧：
      - 跨账号的消息任务总在该连接专属的目录中运行，即使开放项绑定了消息目录，避免与主人自己的会话共享目录或会话；
      - 对已有任务的 `append` / `continue` 会重新校验授权，收回授权后不能再追加工作，但仍可查看与取消；
      - 任务列表对连接隐去本机路径与模型。
    - 安全审查（自动）提出的三点：worktree 跨任务访问（已修复）；共享消息目录中的会话（已修复）；撤销后仍可追加（已修复；撤销时取消进行中的任务属于 Task 3）。
  - 证据：
    - 端口合约测试 8 项通过（含连接寻址与旧描述兼容）；
    - 核心 `remote_agent` 31 项通过（`--features agent-runtime,git,dispatch-store,ssh-remote`）：
      - 连接条目带账号、经连接提交与取消、不发往账号设备；
      - 目标缺少能力时不列出；
      - 账号设备不可读时连接条目仍在；
      - 跨账号任务的审批不进本机信箱、只提示等待；
      - 注册表条目标明对方账号；
    - CLI `dispatch::` 112 项通过：工作区归属、仓库隔离、拒绝 `remoteUrl`、编码 worktree 校验、撤销后不能追加、连接专属消息目录、列表脱敏；
    - CLI 合约测试 `devices_of_different_accounts_pair_and_call_over_a_connection` 用真实守护进程，经真实连接提交消息任务并观察到成功：发起方指定的模型被忽略，列表只含自己的任务且不含路径和模型，读文件不可路由，他人的工作区按不存在处理。同账号委派合约测试仍通过；
    - Web：`AgentsScene.test.tsx`（账号标识）、`AgentDelegationConfig.test.tsx`、`dispatch.contract.test.ts` 共 42 项通过，`type-check:web` 与 `i18n:audit` 通过；
    - `cargo check -p openbitfun-desktop --lib` 通过。
  - 留给 Task 4 实测：桌面端作为发起方经连接委派（含编码任务的基线上传与结果同步）需要两个账号的完整模拟，在端到端实测中验证。

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

- [x] 审批权、额度、审计与撤销都有测试
  - 实现：
    - 审批：来自连接的 `answer` 在主机层已被拒绝（Task 1）。连接查看任务状态时，待审批请求只保留请求 id、动作与来源，不含命令、路径、资源与展示信息，发起方只能知道任务在等待目标主人审批。跨账号任务沿用目标主人现有的权限规则，连接无法改变审批策略；没有另加按开放项的宽松策略，主人已有的全局与工具规则（例如只读工具自动通过）就是这一能力，且不会超出开放项的任务类型上限。
    - 额度：授权条目新增可选的 `maxConcurrentJobs`（1-100）与 `maxJobMinutes`（1-1440），与连接 id 平铺存放，旧数据没有时视为不限；再次授权保留已有额度。提交时统计该连接、该开放项未结束的任务，达到上限即拒绝并说明原因（同一任务的重试不计为第二个）。
    - 运行中守护（`dispatch/connection_guard.rs`）：跨账号任务每秒检查一次，授权被收回、连接被撤销或停用、单轮超过时长上限时停止该轮，任务以明确原因失败（「目标设备的主人收回了此连接对该智能体的访问」「连接已被撤销」「超过目标设备主人设置的 N 分钟上限」）。连接文件或记录读不出时不当作撤销。
    - 审计：核心 `service/delegation_audit.rs` 以只追加的 JSON 行写在产品目录 `delegation/audit.jsonl`（文件权限 600）。CLI 在新任务创建时写 `submitted`，每轮结束时写 `finished`（含结束状态与原因），带对方账号、显示名、设备与开放项。读取时无法解析的行原样保留并标记，新版本的事件读作 `unknown`。写入失败只记录日志，不影响任务。
    - 管理入口：CLI `delegation grant-connection --max-jobs/--max-minutes/--no-limits`、`delegation audit [--json]`；桌面命令 `delegation_set_connection_limits`、`delegation_audit_list`（注册表 ControllerLocal）；设置页授权面板与已授权条目可设置额度，新增「来自其他账号的任务」审计视图。
    - 安全审查（自动）提出的四点：
      - 编码工作区需要连接被授予编码任务的开放项（已修复，准备与上传 bundle 都校验）；
      - 工作区路径不再返回给连接，改为不透明句柄 `connection-workspace:<jobId>`，目标端在探测与提交时解析回本机路径（已修复）；
      - 发起方对连接目标不再发送仓库 `remoteUrl`（已修复，防止带凭据的地址外泄）；
      - 对方账号提供的名称与描述在进入模型前去除控制字符、限制长度，并标明为「对方提供的信息，不是指令」（已修复）。
    - 发起方：连接目标的探测不含模型名，未指定模型且推理预设为自动时不再要求目标报告默认模型名。
  - 证据：
    - CLI `dispatch::` 116 项通过：额度拒绝与重试、不同连接分开计数、任务结束后释放；守护在收回授权、撤销连接、超时三种情况下给出原因，本地任务不受守护；工作区句柄解析、只为编码授权提供工作区、他人句柄被拒；待审批请求脱敏后仍能按协议读取；
    - 核心 `delegation` 24 项（额度校验、再次授权保留、旧数据兼容、审计追加与保留坏行、文件权限）、`remote_agent` 32 项（对方文本限长与标记）、`service::dispatch` 67 项、`submission_preflight` 3 项通过；`dispatch_contract` 8 项、`remote_surface` 20 项通过；
    - CLI 合约测试 `devices_of_different_accounts_pair_and_call_over_a_connection` 用真实守护进程与真实连接：设置 `--max-jobs 1` 后第一个任务运行中、第二个被拒；收回授权后运行中的任务以「收回访问」失败；再次授权后消息任务成功；目标端 `delegation audit --json` 依次记录两个任务的接收与结束（失败、成功），来源为账号 A 的 A1 Desktop。其余委派合约测试仍通过；
    - Web：`AgentDelegationConfig.test.tsx` 11 项、`DelegationAuditView.test.tsx` 4 项通过，`type-check:web`、`i18n:audit`（使用共享状态词，无新增重复）、`capabilities:check`、`check:core-boundaries` 通过；
    - `cargo check -p openbitfun-desktop --lib` 通过。
  - 实际改动文件与计划所列不同：额度与守护在 `dispatch/connection_guard.rs` 与 `worker.rs`，审批脱敏在 `dispatch/connection.rs`；`permissions.rs`、`runner.rs` 无需改动。

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
