---
id: "F002-12"
title: "使用共享的助理：直接对话、对话列表、审批与排队状态、离线展示"
type: "feature"
status: "approved"
summary: "使用者点开共享给自己的助理即可对话：每个对话是主人那边的一个外部任务，追问延续同一会话，经局域网通道提交、按游标补收；对话列在该助理下面，保留期内可回到旧对话继续；需要主人审批或排队时如实显示；主人不在线时置灰，可看不可发。"
source: "docs/superplan/human/features.md"
created: "2026-11-02"
order: 12
depends_on: ["F002-11"]
parent: "F002"
---

# 使用共享的助理 Plan

**需求基准：** RFC `docs/superplan/rfcs/F002.md` 第 6 版「AI协作需求（基准，第 6 版）」第 4、5 条；D3、D7、D8、D9（多租户工作空间）、D11；「交互设计要点」的状态与出错；批准条件 3、4、5、6（进行中任务被取消、不能再发送）、7、9。

**Goal:** B 点开 A 共享的助理 X 就能对话，体验与和自己的助理对话一致；B 的所有对话都在 X 名下 B 的工作空间里，文件可以跨对话延续；A 在 X 名下按使用者分组看到这些会话并在需要时审批；B 关闭应用再打开可以回到旧对话继续；A 不在线时 B 看得到原因和已收到的内容。

**Scope:**
- **使用者：对话**
  - 点开来自共享的助理卡片进入对话界面，与自己的助理对话使用同一套聊天组件：发送、流式回复、工具卡片、子智能体、附件（主人设置允许的类型）。
  - 每个新对话是主人那边的一个外部任务（Dispatch 委派任务，`remote` 审批策略）；回复进行中发送的消息是 `append`，结束后发送的消息是 `continue`，延续同一会话。
  - 对话列表在该助理下面（导航中与自己的助理同样展开），显示标题、最后时间与状态；可以回到旧对话继续，直到超过主人设置的保留天数。
  - 状态如实显示：「等待 某某 审批」（主人那边有待处理的权限请求时）、「排队中，前面还有 N 个」（超出同时任务数时）、「已停止」「已撤销」「已过期」。
  - 模型由主人那边决定，使用者端不显示模型选择；审批策略固定，不显示审批选择。
- **使用者：离线与失败**
  - 主人设备不在局域网中（发现中丢失且连接失败）时，卡片与对话置灰并显示「不在线」；已收到的内容可以查看，输入框不可发送并说明原因。
  - 重新出现时自动按游标补收，不需要用户操作。
  - 每种失败一句原因和下一步：不在线（等待对方上线）、需要更新（更新 OpenBitFun）、已撤销（重新申请）、已过期（开始新对话）、超出保留期继续旧对话时明确失败。
- **使用者：移除**
  - 移除来自共享的助理时，同时删除本地的对话展示副本（F002-11 的移除加上对话记录）。
- **主人**
  - X 名下按租户分组显示会话：「我的」（默认工作空间）与每台使用者设备（「来自 某某的设备」），展开是该设备的会话；会话只读查看 + 审批（沿用 F002-07/09 的只读会话与审批，列表改为按租户分组）。
  - 权限请求进入主人的权限邮箱，桌面端、手机端、IM 都可审批（沿用）；`AskUserQuestion` 明确失败（沿用）。
  - 主人撤销或停止共享时，进行中的任务被停止（F002-11 已实现），使用者的对话显示「已撤销」且不能再发送。
- **执行器**
  - 局域网主机把来自已授权设备的 Dispatch 目标命令转给安装包内的执行器，并绑定该设备的共享公钥（`__sourceShareKey`）；执行器缺失时，共享入口与对话都显示不可用并给出原因（RFC D9）。

**Non-Goals:**
- 助理之间相互调用（F002-13）。
- 使用者端选择模型或审批策略；主人以外的人审批。
- 把主人那边的工作结果同步回使用者的代码仓库（共享助理的任务不使用 Git 交付）。
- 手机端、IM、CLI 上与共享助理对话（RFC D11）。

**Architecture:**
- **一种新的 Dispatch 目标，不是一套新的对话系统**：
  - core `DispatchTarget` 增加 `SharedAssistant { ownerDeviceId, assistantId }`：无工作区交付（不建基线工作树、不同步结果），提交时带 `delegation`（共享助理 id 与来源说明）与 `remote` 审批策略。
  - 传输：`service/lan_share/dispatch_rpc.rs` 实现现有的 `DeviceDispatchRpc`（`invoke(device_id, command, args)`），经局域网通道向主人设备发送 `dispatch_target_*` 请求；连接钉住 `shared_with_me` 中保存的主人公钥。连接按设备复用，空闲后关闭。
  - 出站记录与观察端沿用 Dispatch：`dispatch_status` 游标补收、`dispatch_continue`、`dispatch_append`、`dispatch_cancel`、`dispatch_query`、转录缓存与权限投影都按出站记录里的目标路由，不复制一套。
  - 出站记录不带工作区身份；导航按共享助理分组显示这些对话，不出现在任何工作区下面（Dispatch Web UI 不变式 26、28 对这种目标的适用方式写进 `src/web-ui/src/features/dispatch/README.md`）。
- **更快的更新**：主人在任务有新事件时向在线的使用者推送 `job_updated {jobId}`，使用者的观察端立即按游标拉取；推送丢失时由现有的定时拉取兜底。推送只是提示，不携带内容。
- **主人端准入**：
  - `service/lan_share/host.rs` 对已授权的对端开放 `dispatch_target_probe`、`submit`、`status`、`continue`、`append`、`cancel`、`query`、`list_jobs`；其余命令拒绝。
  - 桌面端 `dispatch_host::dispatch_for_share_key(command, args, share_key)` 替代 `dispatch_for_connection`，绑定 `__sourceShareKey`，复用 F002-11 的 CLI 准入（只能操作自己发起的任务）。
  - 共享助理的 id 在主人端解析为助理工作区，由执行器在准入时决定，不接受调用方提供的路径（沿用 `__assistantWorkspace`）。
- **在线状态**：使用者端由发现会话（F002-11）在后台低频保持：有来自共享的助理时浏览局域网；`Found` 置为在线，`Lost` 后连接失败置为不在线。无共享给我的助理时不浏览。
- **远程场景**：
  - Peer Device Mode：共享助理的卡片与对话显示「对端模式下不可用」，不回退本地。
  - 远程控制：主人的审批照常经权限邮箱到手机端与 IM；使用者侧手机端与 IM 无入口。
  - Detached Dispatch：协议版本与动词不变；`SharedAssistant` 只是新的目标与传输，能力由 `agent_delegation_v1` 与 `assistant_share_v1` 协商。
  - 远程工作区：不涉及。

**Baseline:**
- F002-11 完成后：共享、发现、申请与授权、`shared_with_me`、局域网主机与 `assistant_share_v1` 方法、目标端按设备公钥准入。
- Dispatch 控制端：`core::service::dispatch::{controller, device_controller, target}`、桌面端 `dispatch_api.rs`、Web UI `src/web-ui/src/features/dispatch/`（观察端、出站记录、转录缓存、导航展示）。
- 外部任务主人端：F002-07（以助理身份运行）、F002-08（受阻告知与排队）、F002-09（保留与清理），F002-11 的租户工作空间，`delegation_tasks_api.rs` 与导航中的外部任务列表。

**Exit Criteria:**
- 两台只装 dmg、未登录账号的 Mac：
  1. B 点开 X 发起对话，回答体现 X 的人设；追问延续同一会话；回复进行中追加的消息影响当前回答；
  2. A 在 X 名下「来自 B 的设备」分组里看到这个会话，可以查看；B 开第二个对话，能读到第一个对话在工作空间里留下的文件；X 需要执行命令时 A 收到审批，B 的对话显示「等待 A 审批」，A 允许后继续；
  3. A 把同时任务数设为 1，B 同时发起两个对话，第二个显示排队并在第一个结束后开始；
  4. B 在新对话中问「我们上次聊了什么」，助理能查看 B 之前的对话并回答，但看不到其他使用者或 A 自己的会话；回答中用的是 B 的名字而不是 A 的；B 退出再打开应用，回到旧对话可以继续；把保留天数到期模拟后（调整主人端的任务时间），B 继续旧对话时明确失败为「已过期」；
  5. A 退出应用，B 的卡片与对话 10 秒内显示「不在线」，已收到内容可看、不能发送；A 重新打开后自动恢复并补收；
  6. A 撤销 B，B 进行中的对话停止并显示「已撤销」，不能再发送；B 移除 X 后本地对话记录被删除；
  7. 普通信任程度下，B 的任务及其子智能体读取 A 的其他会话、控制 A 的桌面、创建定时任务均被拒绝；提示中没有记忆与 `USER.md`；A 改为完全信任后可用。
- 新目标的提交、续写、追加、游标补收、取消、过期、撤销、离线均有自动化测试；局域网 RPC 有两主机回环测试。

## Task 1: 局域网上的 Dispatch 目标命令

**Outcome:** 已授权的设备可以经局域网通道向主人设备发送 Dispatch 目标命令，主人端绑定该设备的公钥后交给执行器；未授权或命令不在允许范围内时明确拒绝。

**Files:**
- Create: `src/crates/assembly/core/src/service/lan_share/dispatch_rpc.rs`
- Modify: `src/crates/assembly/core/src/service/lan_share/{host.rs,protocol.rs,mod.rs,tests.rs}`
- Modify: `src/apps/desktop/src/api/{dispatch_host.rs,lan_share_api.rs}`
- Modify: `src/crates/services/services-core/src/dispatch_contract.rs`（`bind_dispatch_source_share_key`）
- Modify: `src/crates/assembly/core/tests/lan_share_flow.rs`

**Change Map:**
- `dispatch_rpc.rs`：`LanDispatchRpc` 实现 `DeviceDispatchRpc`；按主人设备 id 找到 `shared_with_me` 中的公钥，复用或建立连接，`call("dispatch", {command, args})`；离线、需要更新、已撤销各映射为明确错误。
- `host.rs`：方法 `dispatch` 只对已授权对端开放，命令白名单如 Architecture 所列；把命令与对端公钥交给注入的执行回调（桌面端提供 `dispatch_for_share_key`）；任务有新事件时向发起它的对端推送 `job_updated`。
- `dispatch_host.rs`：`dispatch_for_share_key` 绑定 `__sourceShareKey` 后调用执行器。

**Verification:**
- `cargo test --locked -p openbitfun-core --features lan-share --lib service::lan_share`
- `cargo test --locked -p openbitfun-core --features lan-share --test lan_share_flow`
- `cargo test --locked -p openbitfun-desktop --lib api::dispatch_host`
- `cargo test --locked -p openbitfun-services-core --lib dispatch_contract`

- [x] 回环两主机：已授权设备的命令到达执行回调并带正确公钥；未授权、已撤销、白名单外命令被拒绝；`job_updated` 推送到达
  - 证据（2026-11-02）：
    - `cargo test --locked -p openbitfun-core --features lan-share --test lan_share_flow` 5 项通过（连续 5 次）；新增 `a_device_with_access_runs_tasks_on_the_shared_assistant_over_the_channel`（未授权前不发送、提交到达执行回调且 `__sourceShareKey` 为使用者公钥而非调用方自填的值、任务事件文件变化后 `job_updated` 推送到达并发出 `lan_share://job_updated`、`answer` 与工作区命令被拒且未到达执行回调、撤销后本地即返回 `Revoked`）与 `tasks_are_refused_to_devices_without_access_and_offline_owners_are_reported`（陌生设备的 `dispatch` 被拒且未到达执行回调；主人关闭后返回 `Offline`）。
    - `cargo test --locked -p openbitfun-core --features lan-share --lib service::lan_share` 14 项、`--no-default-features --features local-storage` 15 项通过；`cargo +1.95.0 check -p openbitfun-core --features lan-share,dispatch-store,agent-runtime,ssh-remote,git --tests` 通过（`LanDispatchRpc` 随 Dispatch 控制端同一组特性编译）；clippy（1.91.1）在改动文件上无告警。
    - `cargo test --locked -p openbitfun-desktop --lib -- lan_share_api dispatch_host remote_workspace_policy peer_host_invoke` 31 项通过（新增 `a_shared_assistant_command_is_bound_before_any_runner_is_looked_up`）；`cargo test --locked -p openbitfun-services-core --lib dispatch_contract` 7 项通过；`pnpm run check:core-boundaries` 通过。
    - 文档：`docs/architecture/detached-task-dispatch.md` 增加局域网 `dispatch` 方法、执行回调、`job_updated` 与使用者端错误类型的说明。
  - 与计划的差异：
    - `services-core` 的 `bind_dispatch_source_share_key` 已在 F002-11 Task 2 实现，本任务直接复用，不再修改该文件。
    - 主机按「该设备至少持有一个授权」放行 `dispatch`，具体能用哪个助理、能操作哪些任务由执行器按绑定的公钥判定（F002-11 的 CLI 准入）。
    - `job_updated` 由主人端观察该任务的 `events.ndjson` 大小与 `job.json` 修改时间（每 0.5 秒，安静 15 分钟后停止）产生，不依赖执行器回调；推送只是提示。
    - 使用者端错误为 `ShareDispatchError`（未共享、已撤销、不在线、需要更新、被拒绝），`LanDispatchRpc` 以 `anyhow` 包装，调用方可区分。

## Task 2: `SharedAssistant` 目标与对话记录

**Outcome:** core 的 Dispatch 控制端可以向共享助理提交对话、续写、追加、补收、取消与列出；出站记录不带工作区，按共享助理分组；过期、撤销、离线返回明确状态。

**Files:**
- Modify: `src/crates/assembly/core/src/service/dispatch/{target.rs,device_controller.rs,controller.rs,mod.rs}`
- Create: `src/crates/assembly/core/src/service/dispatch/shared_assistant.rs`
- Modify: `src/apps/desktop/src/api/dispatch_api.rs`
- Modify: `src/web-ui/src/features/dispatch/{types.ts,dispatchApi.ts,dispatch.contract.test.ts}`

**Change Map:**
- `target.rs`：`DispatchTargetRequest::SharedAssistant` 与 `DispatchTarget::SharedAssistant`，无工作区交付。
- `shared_assistant.rs`：提交时组装委派任务（共享助理 id、来源会话、`remote` 审批策略、`__assistantWorkspace` 由主人端决定）；续写与追加沿用 `device_controller` 的路径；目标端返回任务不存在且出站记录早于保留期时报告「已过期」。
- `dispatch_api.rs`：`dispatch_submit` 等命令接受新目标并使用 `LanDispatchRpc`；`dispatch_list_jobs` 可按共享助理列出。
- 出站记录的工作区身份对此目标为空；基线工作树、同步结果、`includeUncommitted` 对此目标不可用并在类型上排除。

**Verification:**
- `cargo test --locked -p openbitfun-core --features lan-share --lib service::dispatch::shared_assistant`
- `cargo test --locked -p openbitfun-core --lib service::dispatch`
- `pnpm --dir src/web-ui run test:run src/features/dispatch/dispatch.contract.test.ts`

- [x] 提交、续写、追加、补收、取消、列出、过期、撤销、离线均有测试；对既有 SSH 与设备目标的测试无变化
  - 证据（2026-11-02）：
    - `cargo test --locked -p openbitfun-core --features lan-share,dispatch-store,agent-runtime,ssh-remote,git --lib service::dispatch` 65 项通过，其中 `shared_assistant` 8 项：提交组装委派任务（`remote` 审批、所有者目录中的智能体类型与名称、无工作区与模型、无基线）；提交前拒绝非 `delegation-` 任务号、非 `remote` 审批、`includeUncommitted`、`baseRef`、空助理且不发送任何请求；需要更新、助理不可用、已撤销（目录中没有）、不在线；提交无应答记为 `submission_unknown`；补收、续写（去掉模型、推理强度与审批策略）、追加、取消到达主人；任务不存在记为 `expired`、主人的授权拒绝转为 `revoked`；按助理列出并建立出站记录、不接管他人的任务号、移除后删除记录；审批不走共享助理。既有 SSH 与设备目标测试全部通过。
    - `service::lan_share::dispatch_rpc` 1 项（通道错误转为 `SharedAssistantProblem`）；`--test lan_share_flow` 5 项通过。
    - `cargo test --locked -p openbitfun-cli --bin openbitfun dispatch::` 114 项通过（`share_call`：向使用者列出任务时去掉工作区路径与 `assistantWorkspace`）。
    - `cargo test --locked -p openbitfun-desktop --lib -- lan_share_api dispatch remote_workspace_policy peer_host_invoke` 34 项通过；clippy（1.91.1）在改动文件上无告警；`pnpm run check:core-boundaries` 通过。
    - `pnpm --dir src/web-ui run test:run src/features/dispatch/dispatch.contract.test.ts` 16 项通过（新增：目标形状、错误代码与 Rust 一致、错误解析、任务号前缀、`expired` 为结束状态）；`src/features/dispatch`、`NavPanel`、`flow_chat` Dispatch 驱动与存储 68 个文件 902 项通过；`type-check:web` 通过；`lint:web` 0 错误（16 个既有告警）；`i18n:audit` 通过。
    - 文档：`detached-task-dispatch.md` 增加 `sharedAssistant` 目标；`src/web-ui/src/features/dispatch/README.md` 增加不变式 33（不变式 26、28 对该目标的适用方式）。
  - 与计划的差异：
    - 计划中的验证命令 `--features lan-share --lib service::dispatch::shared_assistant` 与 `--lib service::dispatch` 不启用 Dispatch 控制端（0 项），实际使用上面的特性组合；`shared_assistant.rs` 与 `device_controller.rs` 同样需要 `agent-runtime` 与 `ssh-remote`。
    - 过期判定不比较保留期：主人端对有出站记录的任务答复「不存在」即为已过期（保留期清理或主人删除数据），并把出站记录状态记为 `expired`。
    - 失败原因以 `[shared-assistant:<code>]` 标在错误信息开头（`notShared`、`revoked`、`offline`、`needsUpdate`、`expired`、`unavailable`），Web UI 用 `sharedAssistantProblem` 读取；代码清单由契约测试与 Rust 对齐。
    - 共享助理的对话任务号必须以 `delegation-` 开头，主人端的外部任务视图据此列出与审批。
    - `dispatch_answer`、`dispatch_sync_result`、`dispatch_probe_target` 对该目标明确拒绝；`lan_share_remove` 同时删除本地对话记录与转录缓存（计划在 Task 3 范围内，成本低，一并完成）。
    - Web UI 类型：新增 `GitDispatchTargetRequest`/`GitDispatchTarget`（目标选择只提供这两类）、`expired` 状态与导航文案；设备互联概览不计入共享助理的任务。

## Task 3: 使用者界面：对话、对话列表与状态

**Outcome:** 点开来自共享的助理进入对话；对话列在该助理下面；审批、排队、离线、撤销、过期如实显示。

**Files:**
- Modify: `src/web-ui/src/app/scenes/profile/views/SharedAssistantCard.tsx`、`src/web-ui/src/infrastructure/lan-share/lanShareStore.ts`
- Modify: `src/web-ui/src/app/components/NavPanel/sections/`（来自共享的助理及其对话列表）
- Modify: `src/web-ui/src/features/dispatch/{DispatchJobObserver.ts,dispatchJobStore.ts,dispatchNavPresentation.ts,README.md}`
- Create: `src/web-ui/src/features/dispatch/sharedAssistantConversation.ts` 与测试
- Modify: `src/web-ui/src/app/components/NavPanel/sections/workspaces/DelegatedTaskList.tsx` 与测试、`src/apps/desktop/src/api/delegation_tasks_api.rs`（按租户分组）
- Modify: 三种语言的 `flow-chat.json` 与该页命名空间

**Change Map:**
- 卡片点击：有对话时打开最近一个，否则开始新对话；导航中来自共享的助理下列出对话，「新对话」入口与自己的助理一致。
- 观察端：`SharedAssistant` 对话按出站记录路由；收到 `job_updated` 事件时立即补收。
- 状态：权限待处理显示「等待 某某 审批」；排队显示位置；不在线时输入框禁用并说明；撤销与过期显示原因和下一步。
- 输入框不显示模型与审批选择；附件按目标探测到的能力启用。
- 移除共享的助理时删除其出站记录与转录缓存。
- 主人端：助理名下的外部任务列表按租户分组，组标题为「来自 某某的设备」，显示该设备的会话数与最近时间。
- Peer Device Mode 下显示不可用。

**Verification:**
- `pnpm --dir src/web-ui run test:run src/features/dispatch src/app/scenes/profile/views src/infrastructure/lan-share`
- `pnpm run type-check:web`、`pnpm run lint:web`、`pnpm run i18n:audit`、`pnpm run theme:color-audit:all`

- [x] 对话、续写、追加、状态显示、离线不可发送、移除清理有组件测试
  - 实现（694e3f7d0、68d4e387f、a639178c0、52d8a2de8、8eff774c4、2ad5f28d1）：
    - 卡片点击打开该助理最近的对话，没有时开始新对话；卡片加「新对话」与「详情」按钮；移除助理时按删除会话的方式删除本机的对话（投影、出站记录、转录缓存）。入口 `features/dispatch/sharedAssistantConversation.ts`，会话由 `createSharedAssistantSession` 创建，不走工作区解析。
    - 对话不属于本机任何工作区：任务存储与观察端保留并投影无工作区的记录；驱动不发 `baseRef`、`includeUncommitted` 恒为 false，任务 id 用 `delegation-` 前缀；进度标签为「正在发给「某某」…」。
    - 输入框不显示模型、审批与工作树选择（`ownerDecidesOptions`）；助理不在线、已停止共享或已不在列表时禁用发送，并在输入框说明原因（`sharedAssistantBlock`）。带 `[shared-assistant:<code>]` 的失败显示原因与下一步（`sharedAssistantProblemText`）；附件按设备通道预算检查。
    - 导航在自己的助理之后列出「来自共享」：每个助理及其对话（新到旧），状态显示「等待 某某 审批」、排队（目标返回 `hold` 时显示「某某 同时只处理 N 个任务」）、进行中、已过期、失败；可打开最近对话或新对话；Peer Device Mode 下不显示，卡片在该模式下只开详情并说明不可用。
    - 收到主人的 `job_updated` 推送时立即补读该任务，轮询仍作兜底。
    - 主人端：助理名下的外部任务按发来的设备分组，组标题「来自 某某 · 设备」、会话数与最近开始时间。每个任务本已带来源，桌面端命令未改动（与 Files 中 `delegation_tasks_api.rs` 的差异）。
    - 与计划的差异：排队只显示主人的并发额度，不显示「第几位」——目标端 `hold` 只有原因、额度与开始等待时间，没有位置。
  - 验证：`pnpm --dir src/web-ui run test:run src/features/dispatch src/app/scenes/profile/views src/infrastructure/lan-share src/flow_chat/session-drivers src/app/components/NavPanel` 79 个文件 621 项通过（其后的分组与附件改动所在文件复跑通过）；`type-check:web`（`tsc --noEmit`）、改动文件 eslint、`i18n:audit`（未提高基线，「运行中/失败」改用共享术语）、`theme:color-audit:all`、`appearance:contract-audit` 通过。真机对话验收在 Task 5。

## Task 4: 在线状态与推送

**Outcome:** 使用者端在有来自共享的助理时后台保持发现，在线状态 10 秒内反映主人上下线；`job_updated` 推送让回复无需等待定时拉取。

**Files:**
- Modify: `src/crates/assembly/core/src/service/lan_share/{browse.rs,host.rs,mod.rs}`
- Modify: `src/apps/desktop/src/api/lan_share_api.rs`
- Modify: `src/web-ui/src/infrastructure/lan-share/lanShareStore.ts`

**Change Map:**
- 后台发现：有共享给我的助理时浏览，没有时停止；`Found` 在线，`Lost` 后探测连接失败才置为不在线，避免一次丢包就置灰。
- 发出 `lan_share://presence` 事件；Web UI 订阅更新卡片与对话。
- 推送到达时转发为观察端事件。

**Verification:**
- `cargo test --locked -p openbitfun-core --features lan-share --lib service::lan_share`
- `cargo test --locked -p openbitfun-core --features lan-share --test lan_share_flow`

- [x] 主人主机停止后使用者端置为不在线，重新出现后恢复；推送触发补收
  - 证据（2026-11-02）：
    - 起因：F002-11 Task 7 第一轮验收中，关闭「发现」后主人通道被关闭，撤销推送 `delivered=false`，使用者看不到撤销。因此本任务提前到 Task 3 之前完成。
    - `host.rs`：Bonjour 浏览改为两个理由之一即运行，即「发现」面板打开，或有可用的来自共享的助理（`keep`）。启动时与来自共享的列表每次变化后同步一次。面板关闭时，若仍需保持，只关闭既无进行中申请、又不属于来自共享助理主人的通道；后台只连接这些主人，其他设备等面板打开时再读目录。面板打开时重读所有已发现设备的目录。通道打开或关闭（`LanConnection::closed()`）都会发出 `lan_share://shared_with_me`；`shared_with_me_now()` 给每个条目带上 `online`。
    - 桌面端 `lan_share_shared_with_me` 返回带 `online` 的条目；Web UI `SharedWithMe.online`，卡片在主人不可达时显示「不在线」；`LanShareAPI.onJobUpdated` 订阅 `lan_share://job_updated`，供 Task 3 的对话界面补收。
    - 桌面端日志把 `mdns_sd` 限到 INFO：验收中其 DEBUG 约每 3 分钟写满 10 MB，在「保留 2 个备份」的轮转下把有用日志挤掉。
    - 测试：新增回环测试 `the_owner_of_an_assistant_shared_with_this_device_stays_in_reach`，覆盖面板关闭后撤销仍送达、无可用助理后放开通道、主人退出后变为不在线并发出事件。`cargo test --locked -p openbitfun-core --features lan-share,dispatch-store,agent-runtime,ssh-remote,git --test lan_share_flow` 6 项，连续 5 次通过；`--features lan-share --lib lan_share` 15 项通过；`cargo test --locked -p openbitfun-desktop --lib api::lan_share_api` 2 项通过；Web UI 相关 11 个文件 61 项通过（含「不在线」卡片测试）；`type-check:web`、`lint:web`（0 错误）、`i18n:audit`、`theme:color-audit:all`、`check:core-boundaries` 通过。
  - 与计划的差异：
    - 不新增 `lan_share://presence` 事件与命令：在线状态作为来自共享条目的 `online` 字段，随已有的 `lan_share://shared_with_me` 事件刷新，少一个命令及其注册表行。
    - 在线由到主人的通道是否打开决定：Bonjour `Lost` 只更新发现列表，通道仍在时不置为不在线；主人退出时通道关闭，随即置为不在线。主人重新出现时 `Found` 会重新连接并读目录，读目录同时补上离线期间的撤销。
    - 计划的验证命令 `--features lan-share --test lan_share_flow` 编译不了该测试，需要 `lan-share,dispatch-store,agent-runtime,ssh-remote,git`（与 Task 1 记录相同）。

## Task 5: 两台 Mac 真机验收

**Outcome:** 按 Exit Criteria 在两台只装 dmg、未登录账号的 Mac 上走完对话、审批、排队、重启后继续、过期、离线、撤销、移除与信任程度。

**Files:**
- Modify: `docs/superplan/plans/features/F002/12-use-shared-assistants.md`（证据）

**Change Map:**
- 打 arm64 与 x64 dmg 分别安装；用 cua-driver 驱动两台 Mac 的界面；保留期到期用主人端修改任务时间模拟，并在证据中说明。

**Verification:**
- 手工：Exit Criteria 第 1–7 条逐条记录。

- [ ] 两台 Mac 真机验收逐条通过，或记录失败与修复

## 风险

| 风险 | 缓解 |
|---|---|
| Dispatch 控制端处处假设有工作区与 Git 交付 | 新目标在类型上排除工作区相关字段；对既有目标的测试作为回归保护；不变式的适用方式写进 README |
| 观察端定时拉取在局域网上显得慢 | `job_updated` 推送提示立即补收，定时拉取只做兜底 |
| 主人端执行器缺失 | 共享入口与对话都显示不可用和原因（F002-10 已随安装包分发执行器） |
| 一次网络抖动就置灰 | 发现丢失后再探测连接，失败才置为不在线 |
| 使用者的本地副本与主人的会话不一致 | 以主人会话为准，按游标补收；截断与遗漏如实显示（Dispatch 不变式 21） |

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/11-share-and-request.md`
- `src/web-ui/src/features/dispatch/README.md`
- `docs/architecture/detached-task-dispatch.md`
