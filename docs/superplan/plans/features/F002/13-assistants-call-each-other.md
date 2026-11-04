---
id: "F002-13"
title: "助理之间相互调用：「助理」类目的子智能体、本机助理任务会话、调用链环检测、不可用时如实列出"
type: "feature"
status: "in_progress"
summary: "主人自己的每个助理，以及别人共享给主人的每个助理，都以「助理」类目出现在可调用的子智能体列表中；助理在后台调用它们并经 AgentWait 收回结果。调用自己的助理时在本机该助理名下建一个标明来源的任务会话；调用共享给我的助理时复用 F002-12 的授权与通道。每次调用携带调用链，出现循环时明确拒绝；不可用的助理仍列出并标明原因。RFC 第 8 版起：调用是持久的工作单元，回合结束与重启后仍可收回、完成时通知发起方；调用在发起方的对话里展示，被调用助理的对话列表不列出调用会话，改在其主页折叠展示。"
source: "docs/superplan/human/features.md"
created: "2026-11-02"
order: 13
depends_on: ["F002-12"]
parent: "F002"
---

# 助理之间相互调用 Plan

**需求基准：** RFC `docs/superplan/rfcs/F002.md` 第 8 版 D10（持久调用与展示，2026-11-04 主人批准）；第 6 版「AI协作需求（基准，第 6 版）」第 6 条；D8（普通程度下不能调用主人的其他助理）、D9（多租户工作空间）、D10；兼容性一节（F002 上线前不考虑兼容）；批准条件 5（调用主人的其他助理部分）、8。

**第 8 版修订（2026-11-04）：** 按主人批准的 RFC 第 8 版，增加 Task 4（持久调用：重启后收回结果、完成时通知发起方）与 Task 5（发起方展示、被调用助理主页的折叠记录、审批与提问送到发起方）；原 Task 4「外部任务的继续调用与计数」暂停，改为 Task 6，在 Task 4、5 之后做，因为计数、审计与撤销都以持久的调用记录为准；真机验收改为 Task 7。Task 1–3 不变。

**Goal:** A 有自己的助理 X、Y 和 B 共享给 A 的助理 Z。在 X 的对话中让它调用 Y 和 Z，两者都在后台完成，X 收回结果并继续；Y 的任务会话在 Y 名下标「来自 助理 X」；构造 X→Y→X 时被拒绝，原因是循环调用；Z 不在线时 X 看到 Z 被列出且标明不在线，调用明确失败。

**Scope:**
- **可调用的助理列表**
  - 子智能体列表增加「助理」类目：主人自己的每个助理（不含发起调用的助理自身），以及共享给主人的每个助理（F002-11 的 `shared_with_me`）。
  - 每项显示助理名、说明与来源（「我的助理」或「来自 某某的设备」）。
  - 不可用的助理仍列出并标明原因：不在线、已撤销、需要更新、对方执行器不可用；调用时明确失败，原因与列表一致（修复 B003 一类「悄悄消失」的问题）。
- **调用方式**
  - 助理通过现有的后台委派调用（`Task` 后台运行），不被阻塞；结果经 `AgentWait` 收回；可取消。
  - 调用自己的助理：在本机该助理名下建一个任务会话，标「来自 助理 X」，可在导航中查看。调用发生在主人自己的会话中时，会话放在被调用助理的默认工作空间，按主人自己的任务处理，能力完整；调用发生在某个租户的外部任务中（仅完全信任）时，会话放在被调用助理名下同一租户的工作空间。
  - 调用共享给我的助理：与 F002-12 的对话使用同一授权与通道；在主人那边是一个标明来源的外部任务；需要对方审批时，调用方的任务卡片显示「等待 某某 审批」，排队时显示排队状态。
- **调用链与环检测**
  - 每次调用携带调用链：参与过的助理及其所在设备的标识（`设备 id/助理 id`）。
  - 目标发现自己已在链中时明确拒绝，原因写明「循环调用」，并列出链路。这是结构性的检测，不按次数或字符串限制（遵守 AGENTS.md「Agent loop behavior」）。
  - 本机调用在提交前检查；跨设备调用由目标端执行器在准入时再检查一次，不依赖调用方。
- **外部任务的继续调用（D8）**
  - 普通信任程度：外部任务及其子智能体看不到「助理」类目，直接调用也在工具层被拒绝，原因是「这个共享不允许调用其他助理」。
  - 完全信任：外部任务可以调用主人的其他助理与共享给主人的助理；由此产生的任务计入原共享的同时任务数，并在审计中记在原使用者名下。
- **持久调用（第 8 版）**
  - 本机调用沿用协调存储（`agent-runtime/coordination.sqlite` 的 `background_tasks`）作为调用记录，不另建一套：发起方回合结束后调用继续，任何后续回合与重启后都能收回结果。
  - 调用完成、失败或需要审批时，发起方的对话里出现通知；发起方下一回合的上下文中列出尚未收回的已结束调用，提示用 `AgentWait` 读取。不自动开始发起方的新回合（RFC 第 8 版非目标）。
  - 重启后，仍标为运行中的本机调用在启动时（而不是下一次 `AgentWait` 时）明确标为「应用重启中断」，调用卡片与发起方都能看到这个结果。
  - 发起方的会话被删除时，进行中的调用一起取消。
- **展示（第 8 版）**
  - 发起方的调用卡片写被调用助理的名字与归属、状态与结果；点开在发起方的侧栏中查看被调用助理的过程，不离开当前对话，也不把它加进被调用助理的列表。
  - 被调用助理的对话列表不列出带 `delegatedBy` 的会话；该助理主页有一个默认折叠的「为其他助理做的任务」，列出调用（谁调用的、何时、状态），点开只读查看。
  - 被调用的本机助理需要审批或提问时，请求显示在发起方的输入区审批条与调用卡片上，并经权限邮箱到达手机端与 IM。
- **不在范围内的调用方**：主人直接在普通（非助理）会话中的智能体也能看到「助理」类目，行为相同。

**Non-Goals:**
- 跨设备的调用链可视化图；只在会话与拒绝原因中显示链路。
- 对他人共享给我的助理做并发配额（由对方的同时任务数决定）。
- 手机端、IM、CLI 主机上发起助理之间的调用（CLI 主机不提供共享，RFC D11）。
- 小应用、定时任务、IM、webhook、邮件发起调用；调用完成后自动开始发起方的新回合（RFC 第 8 版非目标，以后单独立项）。
- 恢复运行被重启打断的本机调用：明确标为中断，发起方可以重新调用。

**Architecture:**
- **地址与列表**：
  - 自己的助理：`assistant:<目录名>`（沿用 F002-07 的 `assistant_agent_type`）。
  - 共享给我的助理：`shared:<主人设备 id>/<助理 id>`，替代第 5 版的 `remote:` 与 `conn:` 地址；`runtime-ports::remote_agent` 的地址解析与说明改为这一形状。
  - core `agentic/agents/registry/assistants.rs` 提供「助理」类目：读取本机助理目录与 `shared_with_me`，合并在线状态（F002-12 Task 4），生成带来源与不可用原因的条目。
- **调用自己的助理**：
  - 新增 `agentic/coordination/assistant_turn.rs`：按调用方所属的租户选择目标助理的工作空间（主人 → 默认工作空间；租户 → 该助理名下同一租户的工作空间，不存在则创建），创建会话并标记 `delegatedBy {assistant, sessionId, callChain}`；运行在执行引擎内，作为后台子智能体，结果经现有的后台结果与 `AgentWait` 收回。
  - 主人的调用：会话在目标助理默认工作空间，人设、记忆与工具与该助理自己的会话一致，不受共享设置约束。租户的调用：会话在租户工作空间，沿用外部任务的规则（不参与记忆整理、助理工作区只读）。
- **调用共享给我的助理**：
  - `service/lan_share/remote_agents.rs` 用 F002-12 的 `LanDispatchRpc` 实现 `RemoteAgentDelegationPort`（`list_agents`、`submit`、`observe`、`cancel`、`answer`、`continue_job`），保留 `remote_agent_turn` 的观察、审批提示与排队提示。
  - 提交的委派块带 `callChain`；执行器准入时检查链路并拒绝循环。
- **调用链**：
  - 链路存在会话的委派元数据中：普通会话为空；`assistant_turn` 创建的会话为父链 + 父助理；外部任务的会话为提交时的链（执行器写入）。
  - 每个条目是 `{deviceId, assistantId}`，本机条目使用本机的共享设备 id（F002-10 的身份），因此跨设备的链路可以比较。
  - 链长度不设额外上限；环检测足以阻止无限递归。被调用的助理会话与该助理自己的会话一样可以再调用子智能体与其他助理，不套用「子智能体不能再派生」的深度规则：同一条链上每个助理至多出现一次，链长不会超过参与的助理数（2026-11-03 人类决定，取代原来「嵌套深度沿用现有子智能体深度规则」的写法；否则本机 X→Y→X 根本无法发生，Exit Criteria 第 3 条的循环拒绝无从验证）。
- **外部任务与信任程度**：
  - 外部任务的会话带共享 id 与信任程度（F002-11）；普通程度下，注册表不为它生成「助理」类目，`Task` 工具在执行前再检查一次。
  - 完全信任下由外部任务派生的本机助理会话属于同一租户：放在被调用助理名下该租户的工作空间，继承原共享 id；执行器把它计入该共享的同时任务数，审计记在原使用者名下；撤销该共享时一起停止。
- **远程场景**：
  - Peer Device Mode：「助理」类目在对端模式下列出对端主机自己的助理与共享，由对端执行；控制端不回退本地。
  - 远程控制：手机端与 IM 驱动的会话可以调用助理；跨设备审批照常经权限邮箱。
  - Detached Dispatch：委派块新增必填字段 `callChain`。F002 上线前不考虑兼容，不另设能力协商；局域网通道的协议版本不一致时已显示「需要更新」。
  - 远程工作区：调用自己的助理时在本机助理工作区运行，不涉及 SSH 工作区。

**Baseline:**
- F002-12 完成后：`LanDispatchRpc`、`SharedAssistant` 目标、在线状态、执行器按共享公钥准入。
- `coordination/remote_agent_turn.rs`（观察、审批与排队提示，与传输无关）、`RemoteAgentDelegationPort`。
- 子智能体注册表 `agentic/agents/registry/`、`Task` 与 `AgentWait` 工具、后台结果 `background_outcomes.rs`。
- F002-07 的助理目录与 `assistant:<dir>` 类型、`DelegationTrust` 与工具限制。
- 缺陷 B003（设备离线时没有提示）：第 5 版的中继路径在 F002-11 已删除；本计划保证新的局域网路径上同类情况有提示。

**Exit Criteria:**
- 两台只装 dmg、未登录账号的 Mac：A 有 X、Y，B 共享 Z 给 A：
  1. 在 X 的对话中让它分别调用 Y 和 Z：两者后台完成，X 收回结果并继续；Y 的会话在 Y 名下标「来自 助理 X」；Z 的任务在 B 的 Z 名下标「来自 A 的设备」；
  2. Z 需要 B 审批时，X 的任务卡片显示「等待 B 审批」；
  3. 让 Y 再调用 X（X→Y→X）被拒绝，原因是循环调用并列出链路；跨设备构造 X→Z→（B 的 W，W 完全信任且调用 A 共享给 B 的 X）同样被拒绝；
  4. B 退出应用后，X 的「助理」列表中 Z 仍列出并标「不在线」，调用明确失败；
  5. 普通信任程度下，B 使用 A 共享的 X 时，X 的任务看不到「助理」类目，直接调用被拒绝；A 改为完全信任后可以调用 Y，Y 的会话出现在 Y 名下「来自 B 的设备」分组，该任务计入共享的同时任务数并在审计中记在 B 名下。
  6. （第 8 版）X 的调用卡片写「Y · 我的助理」「Z · 来自 B 的设备」，点开在侧栏看到过程；Y 的对话列表里没有这次调用，Y 主页「为其他助理做的任务」里有一条「来自 助理 X」，可以只读查看；Y 需要审批时在 X 的对话里审批；X 的回合结束后 Y 才完成时，X 的对话里出现完成通知，X 下一回合用 `AgentWait` 读到结果；调用进行中重启 A，调用卡片显示「应用重启中断」，重启后已完成的调用仍能收回结果；X、Y 说起对方时用名字与归属，不显示类型。
- 列表、地址解析、本机助理会话、调用链传递与环检测、信任程度拦截、计数与审计、不可用原因均有自动化测试。

## Task 1: 「助理」类目与地址

**Outcome:** 子智能体列表显示主人的助理与共享给主人的助理，带来源与不可用原因；地址形状 `assistant:` 与 `shared:` 可解析。

**Files:**
- Modify: `src/crates/contracts/runtime-ports/src/remote_agent.rs`、`src/crates/contracts/runtime-ports/tests/remote_agent_port_contracts.rs`
- Create: `src/crates/assembly/core/src/agentic/agents/registry/assistants.rs`
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/{mod.rs,query.rs,types.rs}`、`src/crates/assembly/core/src/agentic/tools/implementations/task/{mod.rs,tests.rs}`
- Modify: `src/web-ui/src/app/scenes/agents/`（类目显示）与语言文件

**Change Map:**
- `remote_agent.rs`：地址改为 `shared:<deviceId>/<assistantId>`，说明改为局域网共享；不可用原因增加 `Revoked`、`ExecutorMissing`。
- `assistants.rs`：生成「助理」类目条目；排除发起调用的助理自身；外部任务在普通信任程度下不生成。
- `Task` 工具：说明中列出助理类目与不可用原因；调用不可用的助理时返回列表中的原因。

**Verification:**
- `cargo test --locked -p openbitfun-runtime-ports --test remote_agent_port_contracts`
- `cargo test --locked -p openbitfun-core --features lan-share --lib agentic::agents::registry`
- `cargo test --locked -p openbitfun-core --lib agentic::tools::implementations::task`

- [x] 类目条目、来源、不可用原因、排除自身、普通信任程度下不生成均有测试

**证据（2026-11-03）：**
- 地址：`RemoteAgentAddress` 改为 `shared:<ownerDeviceId>/<assistantId>`（字段 `assistant_id`，线上名 `assistantId`），不可用原因增加 `revoked`、`executor_missing`；`cargo +1.95.0 test --locked -p openbitfun-runtime-ports --features remote-agent-port --test remote_agent_port_contracts` 5 通过（该测试目标需要 `remote-agent-port` 特性，计划中的命令缺这一项）。
- 类目：`registry/assistants.rs` 取代 `remote_agents.rs`。本机助理来自工作区服务的助理工作区（名称与界面一致，说明取 `IDENTITY.md` 的 vibe，与共享时发给对方的说明相同）；共享给我的助理来自 `RemoteAgentDelegationPort::list_agents`。可用的在前，不可用的排在后面并在说明中写「Unavailable now: …」。`AssistantCaller::of_turn` 按会话所在的助理工作区（外部任务按委派事实中的助理工作区）排除自身，外部任务普通信任程度下整个类目不生成。
- 条目形状：`AgentInfo` 新增可选字段 `assistant {assistantId, owner?, availability}`，替代 `remoteAgent`；没有新增 `AgentSource`/`SubAgentSource` 枚举值，旧版本的对端读到新列表不会因未知枚举值失败。
- `Task`/`AgentSpawn`：说明改为列出 `assistant:<id>` 与 `shared:<device_id>/<assistant_id>`；目录与准入共用 `ensure_assistant_admissible`（父会话限制、普通信任程度拒绝「this share does not allow calling other assistants」、只能后台、不能指定模型）；调用不可用的共享助理返回列表中的原因，例如 `assistant shared:laptop/writer is unavailable (offline): Laptop is offline`。原来「委派来的回合一律不能再调用」的拒绝改为按信任程度。
- 界面：智能体页来源筛选新增「助理」（共享词条 `shared:agents.Claw`），详情显示「我的助理」或「位于 某设备」与可用性（含已停止共享、对方设备无法执行任务）；任务卡片与审批条按新地址显示「（共享的助理）」「（我的助理）」。
- 验证：`cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git --lib agentic::agents::registry` 66 通过；同特性 `--lib agentic::tools::implementations::task` 98 通过、`--lib skill_agent_snapshot` 6 通过；`--features agent-runtime,git,remote-workspace --lib agentic::coordination::remote_agent_turn` 8 通过；`cargo check -p openbitfun-core --features lan-share --tests` 通过（计划中带 `--features lan-share` 的 `--lib agentic::...` 命令在默认特性下不编译 agentic 模块，匹配 0 个测试，改用 core AGENTS.md 中的特性组合）；Web UI `tsc`、`eslint`、`pnpm --dir src/web-ui run test:run src/app/scenes/agents src/flow_chat/utils src/flow_chat/tool-cards/TaskToolDisplay.test.tsx src/flow_chat/components/ChatInputApprovalBand.test.tsx` 89 个文件 860 通过；`pnpm run i18n:audit` 通过。
- 留给后续任务：`assistant:<id>` 的调用在准入后明确返回「cannot be called from this version yet」，由 Task 2 接上执行；共享助理在真机上的列表要等 Task 3 用 `LanDispatchRpc` 实现并注册端口后才出现；完全信任下外部任务调用的计数与审计在 Task 4（第 8 版起为 Task 6）。

## Task 2: 调用自己的助理

**Outcome:** 助理在后台调用本机的另一个助理，目标助理名下出现标明来源的任务会话，结果经 `AgentWait` 收回；调用链随会话传递，本机循环被拒绝。

**Files:**
- Create: `src/crates/assembly/core/src/agentic/coordination/assistant_turn.rs` 与 `assistant_turn/tests.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/{coordinator.rs,mod.rs,delegation_context.rs}`
- Modify: `src/crates/assembly/core/src/agentic/session/`（会话委派元数据：`delegatedBy`、`callChain`）
- Modify: `src/web-ui/src/app/components/NavPanel/sections/`（「来自 助理 X」标记）与语言文件

**Change Map:**
- `assistant_turn.rs`：在目标助理工作区创建会话并运行；写入 `delegatedBy` 与调用链；结束时把最终文本交给后台结果。
- 环检测：提交前比较调用链与目标 `{本机设备 id, 助理 id}`，命中时返回「循环调用」及链路。
- 会话元数据新增可选字段 `delegatedBy`；普通会话没有该字段（会话存储是已发布格式，新字段按 AGENTS.md 以默认值读入）。
- 导航中助理名下的会话显示「来自 助理 X」。

**Verification:**
- `cargo test --locked -p openbitfun-core --lib agentic::coordination::assistant_turn`
- `cargo test --locked -p openbitfun-core --lib agentic::session`
- `pnpm --dir src/web-ui run test:run src/app/components/NavPanel/sections`

- [ ] 后台调用与收回、会话标记、主人与租户调用各自进入正确的工作空间、调用链传递、X→Y→X 拒绝、没有该字段的会话读入均有测试

**进展（2026-11-03，未勾选）：**
- 调用：`Task`/`AgentSpawn` 的 `assistant:<id>` 不再返回「cannot be called from this version yet」。准入后按调用链做本机环检测（设备 id 取 `RemoteAgentDelegationPort::local_device_id`，没有端口时用 `local`），把「原链 + `<设备>/<调用方助理>`」写入 `dispatchDelegationCallChain`，并写入调用方 `assistantCallCaller`/`assistantCallCallerName`。
- 会话：协调器在新建分支中识别助理类型，在目标助理的默认工作区以 `Claw` 新建普通会话（不是隐藏子会话，`continuation_policy = FreshOnly`，名称取任务首行）。会话元数据写入 `delegatedBy {assistant, assistantName?, sessionId, callChain}`，不写子会话关系。子回合上下文去掉调用方的委派事实，只保留调用链。`SubagentSessionLinked` 新增 `assistantSession`（默认 false，旧版本忽略），Web UI 只把任务卡片链接到该会话，不把它改成调用方的子会话。
- 界面：会话列表对有 `delegatedBy` 的会话显示「来自 X」，悬停说明「X 调用这个助理时开始的会话」（zh-CN/zh-TW/en-US）。
- 验证：`cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,lan-share,dispatch-store,ssh-remote --lib -- agentic::tools::implementations::task agentic::coordination` 380 通过（含 `assistant_turn` 3 项、`a_local_assistant_call_carries_its_chain_and_refuses_a_loop_or_a_delegated_caller`、`a_call_to_an_assistant_not_on_this_device_is_refused_before_a_session_starts`）；`--lib -- agentic::session` 204 通过（含 `a_session_another_assistant_started_keeps_its_caller_and_stays_listed`）；`--test lan_share_flow` 12 通过；`openbitfun-events` 32 通过；Web UI `EventHandlerModule.test.ts` 与 `FlowChatStore.test.ts` 325 通过，`tsc`、`eslint`、`pnpm run i18n:audit` 通过。
- 真机（2026-11-04，dmg 8b1248fa，本机隔离实例）：新建「助理 2」（引导后自称小栖），让它调用本机另一个助理（小岚）。它用 `AgentSpawn` 调用 `assistant:workspace`、`AgentWait` 收回，并转述小岚的原话「我是小岚，一个简洁、温暖、可靠的AI助手。」。被调用的会话在小岚的工作区，`sessionKind = standard`，没有父会话，`delegatedBy = {assistant: assistant:workspace-9583265c, assistantName: 小栖, sessionId, callChain: [<设备>/workspace-9583265c]}`。发现并修复：`AgentSpawn`/`AgentWait` 卡片给被调用的助理起了随机子代理昵称（「糯糯」），模型显示「暂未提供模型信息」，现改为显示助理本名与「由该助理自己的设置决定」（95b0008dd）。同时补上安全修复：临时（不保存的）会话调用助理会留下保存的会话，现在在新建前拒绝（dc3cf6878，测试 `a_session_that_is_not_kept_cannot_call_an_assistant`）。
- 未完成（因此未勾选）：外部任务（租户）调用本机助理目前明确拒绝「a task another device delegated cannot call this device's assistants yet」，进入租户工作空间、计数与审计放在 Task 4（第 8 版起为 Task 6）；后台结果经 `AgentWait` 收回沿用现有后台子任务路径，没有专门的端到端测试，留到 Task 5 真机验收（第 8 版起为 Task 7；重启后收回由 Task 4 补测试）；协调器测试只覆盖了目标助理不存在时被拒绝（测试环境不创建真实助理工作区，避免写入用户目录）。

## Task 3: 调用共享给我的助理与跨设备环检测

**Outcome:** 助理在后台调用共享给主人的助理，经局域网在对方那里作为外部任务运行（在主人这台设备的租户工作空间中）；审批与排队提示显示在任务卡片；目标端执行器拒绝循环调用。

**Files:**
- Create: `src/crates/assembly/core/src/service/lan_share/remote_agents.rs`
- Modify: `src/crates/assembly/core/src/agentic/coordination/remote_agent_turn.rs`（地址与提示文案）
- Modify: `src/apps/cli/src/dispatch/{delegation.rs,protocol.rs,mod.rs}`（`callChain` 必填与准入检查）
- Modify: `src/crates/services/services-core/src/dispatch_contract.rs`、`src/web-ui/src/features/dispatch/dispatch.contract.test.ts`
- Modify: `src/apps/desktop/src/lib.rs` 或 `lan_share_api.rs`（注册端口实现）
- Modify: `src/crates/assembly/core/tests/lan_share_flow.rs`、`docs/architecture/detached-task-dispatch.md`

**Change Map:**
- `remote_agents.rs`：用 `LanDispatchRpc` 实现 `RemoteAgentDelegationPort`；列出时合并在线状态与不可用原因；提交时带调用链。
- 执行器准入：委派块中的调用链包含本机 `{设备 id, 助理 id}` 时拒绝「循环调用」；任务会话保存调用链，供它再调用时延续。

**Verification:**
- `cargo test --locked -p openbitfun-core --features lan-share --lib service::lan_share`
- `cargo test --locked -p openbitfun-core --features lan-share --test lan_share_flow`
- `cargo test --locked -p openbitfun-core --lib agentic::coordination::remote_agent_turn`
- `cargo test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `pnpm --dir src/web-ui run test:run src/features/dispatch/dispatch.contract.test.ts`

- [x] 回环两主机：调用、审批提示、排队提示、跨设备循环拒绝、缺少调用链被拒绝、离线失败均有测试
  - 进展（2026-11-03）：
    - 29eb7cf5c：`remote_agents.rs` 的 `SharedAssistantDirectory` 实现端口（列出合并在线状态、撤销与近期拒绝原因；提交前走与对话相同的准入；不写外发记录），桌面端在局域网主机启动后注册；主人端的外部对话列表不再列出助理调用。
    - 调用链：`RemoteAgentSubmitRequest.call_chain` 与端口的 `local_device_id()`；Task 工具准入时用「本轮收到的链 + 本机调用方助理」组链并先在本地查环，经子会话上下文交给远端回合；委派块带 `callChain`，对话提交发送 `[]`；`submit` 缺少该字段时拒绝（已存的任务记录缺少时读为空，保证升级后旧记录仍可读）；执行器检查每项为 `<deviceId>/<assistantId>`；任务回合的 `dispatchDelegation` 元数据与上下文保存收到的链，再调用时延续。
    - 与 Change Map 的差异：目标端的环检测放在主人的局域网主机（转交执行器之前），因为只有它持有本机设备 id；执行器只检查字段存在与格式。行为与计划一致：主人端与请求端各查一次。
    - 测试：`delegation` 链项与环（`call_chains_hold_device_assistants_and_refuse_a_second_visit`）、元数据到上下文、主人端环检测（`a_submission_naming_an_assistant_already_on_its_chain_is_a_loop`）、目录提交带链、CLI 缺少调用链与伪造链项被拒、端口合约的线上字段与旧请求读入。core `--lib`（lan-share,dispatch-store,agent-runtime,ssh-remote,git,remote-workspace）2313 项、`lan_share_flow` 9 项、CLI `dispatch::` 115 项、runtime-ports 合约 6 项通过；`openbitfun-desktop` 检查通过。
    - 回环两主机（2026-11-03）：`lan_share_flow.rs` 的 `assistant_calls` 模块经真实局域网通道和 `SharedAssistantDirectory` + `LanDispatchRpc` 测三个流程：
      - `an_assistant_calls_a_shared_assistant_and_sees_its_queue_and_approvals`：调用带链和来源到达主人，并绑定请求方的密钥；排队原因（`shareLimit`、上限）和待审批读回；请求方不能代答审批。
      - `a_call_that_comes_back_to_an_assistant_on_its_chain_is_refused_by_the_owner`：链上已有主人的助理时，主人在执行器之前拒绝，并给出「circular call」。
      - `a_call_to_an_owner_that_went_away_fails_and_lists_it_offline`：主人离线后列为 Offline，调用失败，主人什么也没收到。
    - 缺少调用链被拒绝：检查在执行器的提交准入（`require_call_chain`）里，由 CLI `dispatch::` 测试覆盖；回环用的是替身执行器，不重复测。
    - `lan_share_flow`（lan-share,dispatch-store,agent-runtime,ssh-remote,git）12 项通过。

## Task 4: 持久调用：重启后收回、完成时通知发起方

**Outcome:** 本机调用的结果在发起方任何后续回合与应用重启后都能收回；调用结束时发起方得到通知，下一回合知道有结果待收；重启打断的调用在启动时明确标为中断；发起方会话删除时调用一起取消。

**Files:**
- Modify: `src/crates/assembly/core/src/agentic/coordination/{background_outcomes.rs,coordinator.rs,assistant_turn.rs}`、`src/crates/assembly/core/src/agentic/session/session_manager.rs`
- Modify: `src/crates/assembly/core/src/agentic/execution/`（发起方回合上下文中列出未收回的已结束调用）
- Modify: `src/crates/contracts/events/src/agentic.rs`、`src/crates/assembly/core/src/.../frontend_projection.rs`（调用结束事件）
- Modify: `src/apps/desktop/src/lib.rs`（启动时整理本机调用）

**Change Map:**
- 结果收回：`load_persisted_result_text` 现在按发起方会话的存储路径读取子会话的回合；助理调用的会话在被调用助理的工作区，重启后（内存结果已清空）读不到。改为按子会话自己的存储位置读取（助理调用由 `delegatedBy` 定位）。
- 启动整理：桌面端启动时对本机调用（没有 `remote_job` 的后台任务）执行 `reconcile_stale_running_tasks`，不再等下一次 `AgentWait`；中断原因写「应用重启中断」。
- 通知：后台任务进入终态时发出调用结束事件（发起方会话、调用编号、被调用方、状态）；Web UI 更新调用卡片，发起方没有进行中的回合时在对话中显示一条通知。
- 下一回合：发起方开始新回合时，上下文中附一条说明，列出已结束但尚未收回的调用（编号、被调用方、状态），提示用 `AgentWait` 读取。这是状态同步，不是按次数或字符串的限制。
- 删除会话：发起方会话删除时取消它进行中的调用。

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,lan-share,dispatch-store,ssh-remote --lib -- agentic::coordination`
- `cargo +1.95.0 test --locked -p openbitfun-events`
- `pnpm --dir src/web-ui run test:run src/flow_chat/services/EventHandlerModule.test.ts src/flow_chat/tool-cards`

- [ ] 重启后从被调用助理的工作区收回结果、启动整理、结束事件、下一回合的说明、删除会话即取消均有测试；旧的后台任务记录（没有新字段）照常读入

**进展（2026-11-04，未勾选）：**
- 3c5810fa0：后台任务新增可空列 `child_storage_path`（原地追加，旧版本照常打开数据库），助理调用在建会话后记下被调用会话的存储位置；`AgentWait` 与过期整理都从那里读回合。原来按发起方的存储读，重启后内存结果清空即读不到。测试 `a_called_assistants_result_is_read_from_its_own_workspace_after_a_restart`。
- c5ea4a3fa：桌面端启动时整理上一进程留下的本机助理调用（只限有 `child_storage_path` 的任务），回合已完成的保留结果，未完成的标为中断「The app restarted before this call finished」；发起方每次开始新回合时，上下文附一条 `BackgroundResult` 类提醒，列出已结束但未收回的助理调用（本机与共享），提示用 `AgentWait` 读取。测试 `a_call_cut_off_by_a_restart_is_settled_at_startup_and_named_to_the_callers_next_turn`（普通子智能体不在整理与提醒之列）。
- 删除会话即取消：删除走会话维护流程，已经调用 `cancel_background_subagents_for_parent_session`，助理调用同在其中，无需改代码。
- 4156f220d（真机发现）：被调用助理只看到任务正文，回答「我没有主人」；现在会话开头有一条提醒，说明来自哪个助理、为同一个人工作、回复交回那个助理。
- 验证：core `--lib -- agentic::coordination agentic::tools::implementations::task` 385 通过；`openbitfun-desktop` 检查通过；legacy-migration-adapters 55 通过。
- 未完成：结束事件与发起方空闲时的界面通知（卡片状态已随被调用会话的事件更新；发起方没有进行中的回合时的单独通知尚未做）。
- 真机（2026-11-04，dev 构建，隔离数据）：小栖调用小岚写《雨后的城市》，进行中结束应用。重启后任务记录为 `interrupted`「The app restarted before this call finished」，`child_storage_path` 已记下；下一回合小栖回答「小岚的撰写任务因应用重启而中断…需要我重新交给她写吗？」。
- afecbc746（真机发现）：重启后被调用会话尚未载入，等待卡片把小岚叫成随机昵称「团团」；现在按调用时请求的类型取名。
- 90281defb（真机发现）：重启后调用卡片读不到被调用会话，把「已开始」当成已结束；现在取发起方会话里最近一次等待记下的结果，真机显示「已取消」。被调用会话在磁盘上仍是进行中，载入时由 Web UI 按中断处理。

## Task 5: 发起方展示、被调用助理主页的折叠记录、审批与提问送到发起方

**Outcome:** 调用只在发起方的对话里展示；被调用助理的对话列表保持干净，主页折叠列出「为其他助理做的任务」；被调用助理的审批与提问在发起方处理。

**Files:**
- Modify: `src/web-ui/src/app/components/NavPanel/sections/sessions/{SessionsSection.tsx,sessionNavHierarchy.ts}` 与测试
- Modify: `src/web-ui/src/app/scenes/profile/views/AssistantConfigPage.tsx`，Create: 该目录下的「为其他助理做的任务」组件与测试
- Modify: `src/web-ui/src/flow_chat/tool-cards/{AgentControlToolCard.tsx,AgentWaitToolCard.tsx}`、侧栏打开会话的路径（`btwSessionPane.ts`）
- Modify: `src/crates/assembly/core/src/agentic/tools/implementations/ask_user_question_tool.rs`（如需要）与语言文件

**Change Map:**
- 导航：`getWorkspaceSessionHierarchy` 不列出带 `delegatedBy` 的会话；去掉列表中的「来自 X」标记（只在主页记录中显示来源）。
- 主页：在定时任务与会话列表之间增加默认折叠的分区，列出该助理工作区中带 `delegatedBy` 的会话（来源、时间、状态），点开在只读模式查看。
- 调用卡片：状态与结果；点开在侧栏打开被调用的会话，不切换工作区；Task 4 的结束事件到达时更新状态。
- 审批与提问：现状下被调用会话的审批以 `delegation.parentSessionId` 归到发起方的输入区；补测试锁定助理调用也如此。提问的控制会话沿父子关系查找，助理调用没有父子关系，改为也认 `delegatedBy.sessionId`；确认权限邮箱在手机端与 IM 中能看到这些请求。

**Verification:**
- `pnpm --dir src/web-ui run test:run src/app/components/NavPanel/sections src/app/scenes/profile src/flow_chat/tool-cards src/flow_chat/components/modern`
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,lan-share,dispatch-store,ssh-remote --lib -- ask_user_question`
- `npx tsc --noEmit -p src/web-ui`、`pnpm run lint:web`、`pnpm run i18n:audit`

- [ ] 列表不列出调用会话、主页折叠记录、卡片侧栏查看、审批归到发起方、提问归到发起方均有测试

**进展（2026-11-04，未勾选）：**
- 13bd28e6a：导航不列出带 `delegatedBy` 的会话，去掉列表里的「来自 X」标记与词条；助理主页在定时会话与会话之间增加默认折叠的「为其他助理做的任务」（只在有记录时出现），点开打开该会话；带 `delegatedBy` 的会话只读（`fresh_only`），由发起方驱动；调用卡片把类型位置换成归属「我的助理」/「来自 某某 的设备」，不再显示 `assistant:`/`shared:` 类型；点开时带被调用会话自己的 `workspaceId`。
- 现状确认：被调用会话的审批带 `delegation.parentSessionId`，Web UI 归到发起方的输入区；提问的控制会话取自调用时的父信息，即发起方。两者待真机确认，尚未补专门测试。
- 验证：Web UI `src/app/scenes/profile src/flow_chat/tool-cards src/flow_chat/utils/remoteAgent.test.ts src/app/components/NavPanel src/flow_chat/session-drivers` 通过；`tsc`、`eslint`、`pnpm run i18n:audit`、`pnpm run theme:color-audit:all` 通过。
- 真机（2026-11-04，dmg b2935109，含 a6d34b04b 与 9cb86e77a）：小栖调用小岚，卡片名为「小岚」（此前为发起方名字「鹅鹅」）；导航不再出现「Remote Session」；小栖能说清可调用的助理各是谁的、在哪里运行，不把小岚当成自己；卡片仍显示 `assistant:workspace` 类型，已由 13bd28e6a 修复，待新包复核。
- eeff3f13d：折叠记录登记外观部件 `calledBy`（release 构建的外观契约检查要求）。faff10e69：可调用助理列表说明调用者不在其中、不能调用自己，谈到助理时说名字、归属与运行位置，不说类型或工作区路径。48a890afe：点导航里的助理打开它自己的最近会话，不打开别人调用它产生的会话。
- 真机（2026-11-04，dev 构建）：小岚的会话列表只有她自己的会话，主页「为其他助理做的任务 · 显示 6 项」列出小栖的调用，调用会话只读；调用卡片显示「小岚 · 我的助理」；小栖只列出小岚与共享助理，不列自己，不显示路径；小岚回答「现在找我的是小栖」。审批与提问送到发起方仍待真机确认。
- 真机（2026-11-04，dev 构建，审批与提问）：小栖切到「需要确认」后调用小岚新建文件，两次审批都出现在小栖的输入区上方，允许后文件写在小岚自己的工作区。da32f286b：审批原写「Claw 子智能体」，改为被调用助理的名字。
- 0f247224a（RFC 第 9 版）：助理（Claw）原本没有提问工具，被调用时只能回报「无法询问用户」；现已加入。真机：小岚提问「你希望这首两行短诗以什么为主题？」出现在小栖的对话里并标明「小岚」，回答「夜」后小岚写完交回。侧栏里小岚调用会话的标题写「小岚 · 来自 当前项目概览」，即发起方会话的标题，符合预期。小栖自己也有了提问工具，要求不明确时它会先问再调用。
- b33de0ce5（RFC 第 9 版）：调用卡片显示被调用会话实际使用的模型（真机「小岚 · 我的助理 · gpt-6.1-terra」）；轮次事件此前只回填 `Task` 卡片，不回填 `AgentSpawn`。运行前本机写「应用默认模型」，共享写「由共享该助理的设备选择」。
- afecbc746、90281defb：重启后等待卡片的名字与调用卡片的状态（见 Task 4 进展）。

## Task 6: 外部任务的继续调用与计数（暂停至 Task 4、5 之后）

**Outcome:** 普通信任程度下外部任务不能调用任何助理；完全信任下可以，派生任务在被调用助理名下同一租户的工作空间运行，计入原共享的同时任务数、在审计中记在原使用者名下，撤销共享时一起停止。

**Files:**
- Modify: `src/crates/assembly/core/src/service/delegation.rs`（工具限制）、`src/crates/assembly/core/src/agentic/coordination/assistant_turn.rs`
- Modify: `src/apps/cli/src/dispatch/{connection_guard.rs,audit.rs,worker.rs}`（按原共享计数与审计）
- Modify: `src/crates/assembly/core/src/service/lan_share/shared_assistants.rs`

**Change Map:**
- 普通信任程度：`Task` 工具对「助理」类目的调用在工具层拒绝，原因明确；注册表不生成该类目（Task 1）。
- 完全信任：派生的本机助理会话继承原共享 id；执行器计数与撤销把它算作原共享的任务；审计条目记录原使用者与派生链路。

**Verification:**
- `cargo test --locked -p openbitfun-core --features lan-share --lib service::delegation`
- `cargo test --locked -p openbitfun-cli --bin openbitfun dispatch::`

- [ ] 普通程度拒绝、完全信任可用、计数、审计、撤销即停止均有测试

## Task 7: 两台 Mac 真机验收

**Outcome:** 按 Exit Criteria 在两台只装 dmg、未登录账号的 Mac 上走完本机调用、跨设备调用、审批提示、循环拒绝、离线与信任程度。

**Files:**
- Modify: `docs/superplan/plans/features/F002/13-assistants-call-each-other.md`（证据）

**Change Map:**
- 打 arm64 与 x64 dmg 分别安装；用 cua-driver 驱动两台 Mac 的界面；跨设备循环需要两边互相共享，按 Exit Criteria 第 3 条准备。

**Verification:**
- 手工：Exit Criteria 第 1–6 条逐条记录。

- [ ] 两台 Mac 真机验收逐条通过，或记录失败与修复

## 风险

| 风险 | 缓解 |
|---|---|
| 助理互相调用形成无限递归 | 结构性调用链检测，本机与目标端各查一次；嵌套深度沿用子智能体规则；不加按次数的限制 |
| 完全信任下外部任务借助理调用绕过同时任务数 | 派生任务继承原共享 id，计数、审计、撤销都按原共享 |
| 本机助理会话写入目标助理的存储与记忆 | 主人自己的调用本就可以；源自外部任务的派生会话放在同一租户的工作空间，沿用外部任务的只读与不参与记忆整理规则 |
| 发起方回合结束后调用无人收回，结果丢失或占用资源 | 结果持久在协调存储，结束时通知发起方并在下一回合提示；会话删除即取消 |
| 调用会话从被调用助理的列表中消失后，主人找不到过程 | 主页「为其他助理做的任务」保留只读记录；发起方卡片可以打开 |
| 列表中混入太多不可用条目 | 不可用条目排在后面并标原因；移除共享的助理后不再列出 |

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/12-use-shared-assistants.md`
- `docs/architecture/detached-task-dispatch.md`
- `AGENTS.md`「Agent loop behavior」
