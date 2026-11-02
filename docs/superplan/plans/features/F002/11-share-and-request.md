---
id: "F002-11"
title: "共享与申请：共享设置、发现面板、倒计时确认框、授权按设备公钥，删除第 5 版 AI协作"
type: "feature"
status: "in_progress"
summary: "主人在「助理」页共享助理并设置信任程度、保留天数与同时任务数；使用者在「发现」面板看到局域网内的共享助理并申请；主人收到 60 秒倒计时确认框，结果经局域网直接推送；授权按使用者设备的共享公钥保存，可撤销、可移除。同时删除第 5 版的 AI协作页、账号连接与中继跨账号能力。"
source: "docs/superplan/human/features.md"
created: "2026-11-02"
order: 11
depends_on: ["F002-10"]
parent: "F002"
---

# 共享与申请 Plan

**需求基准：** RFC `docs/superplan/rfcs/F002.md` 第 6 版「AI协作需求（基准，第 6 版）」第 1–4、7 条；D2（授权绑定设备公钥）、D3（未授权设备只能读目录与申请）、D4、D5、D6、D8（信任程度改为按共享的助理设置）、D9（多租户工作空间）、D11、D12；兼容性一节（F002 上线前不考虑兼容）；「交互设计要点」；批准条件 1、2、6、7（申请与撤销部分）、10。

**Goal:** 两台未登录账号的 Mac 在同一局域网：A 共享助理 X，B 在「发现」中看到并申请，A 在倒计时内允许后 X 立即出现在 B 的助理列表并标明来源；A 可以撤销或停止共享，B 可以移除。第 5 版 AI协作的界面、连接与中继跨账号能力全部删除。

**Scope:**
- **主人：共享助理**
  - 每张助理卡片（含主助理）增加「共享」操作；共享设置：信任程度（普通 / 完全信任，完全信任需确认风险）、会话保留天数（1/7/30/90，默认 7）、同时处理的任务数（1–100，默认 2）。
  - 助理配置了外部智能体时，共享设置里显示一次额度与条款提示，由主人确认；确认记录在该共享上。
  - 共享中的卡片显示「共享中 · N 人在用」；点击打开管理视图：修改设置、查看使用者（显示名、设备名、授权时间、最近使用）、逐个撤销或删除该使用者的数据（租户工作空间）、停止共享。
  - 有共享的助理时才监听与广播；最后一个共享停止后停止广播与监听。
- **使用者：发现与申请**
  - 「助理」页标题栏「新建助理」左边增加「发现」，打开发现面板：按设备分组列出共享助理，悬停显示说明、主人显示名、设备名与指纹短码；状态为 可申请 / 申请中（倒计时）/ 已在你的列表中 / 需要更新。
  - 面板打开时浏览，关闭即停止；面板可修改本机显示名。
  - 面板为空时说明原因：没有发现共享的助理；本地网络权限可能被拒绝（给出打开系统设置的入口）。
  - 申请后显示「等待 某某 确认」和以主人截止时间为准的剩余秒数；结果以轻提示出现：「已添加」或「未获同意」；主人不可达时立即提示「对方不在线」。
- **主人：确认框**
  - 收到申请立即弹出模态确认框：「某某（设备名）想使用你的助理『X』」，显示申请设备的指纹短码；按钮「拒绝」「允许（60）」逐秒倒数，到 0 自动拒绝；可选「拒绝并不再提示」屏蔽该设备。
  - 应用不在前台时同时发系统通知；多个申请排队，各自计时。
  - 防打扰：同一设备对同一助理同时只有一个申请；被拒绝或超时后 1 分钟内不能再申请；被屏蔽的设备的申请直接以「未获同意」结束，不弹框。
- **授权与列表**
  - 授权绑定使用者设备的共享公钥（首次信任）；之后同一设备 id 出现不同公钥，原授权不适用，需重新申请。
  - 被授权的助理进入使用者的「助理」列表，卡片标「来自 某某的设备」；可以移除（撤销授权、删除本地记录，并让主人删除这台设备的租户工作空间）。对话入口在 F002-12 提供，本计划中点击卡片显示该助理的说明与来源。
  - 主人撤销或停止共享时，在线的使用者立即收到推送，卡片标「已撤销」；离线的使用者在下次连上时得知。
- **目标端执行的授权改为按设备公钥**：Dispatch 委派任务的准入、同时任务数与排队、信任程度、会话保留天数、审计、撤销即停止，全部从「连接 id + 按连接设置」改为「设备共享公钥 + 按共享的助理设置」。任务提交与对话在 F002-12 接上局域网通道。
- **多租户工作空间（RFC D9）**：助理的默认工作空间属于主人；每台使用者设备是一个租户，第一次提交任务时由执行器在该助理名下创建这台设备的工作空间，之后它的所有任务都在这里运行、会话都保存在这里。取代第 5 版「每个任务一个独立目录」。助理的身份与人设从助理工作区继承（只读），不运行首次引导；用户画像是租户自己的 `USER.md`（创建时填入使用者显示名与设备名）；会话列表与会话历史工具对外部任务开放，但只作用于本租户的会话。租户记忆不做（RFC 非目标）。租户工作空间在授权不存在且没有会话时删除；使用者移除或主人「删除该使用者的数据」时立即删除；审计记录保留。
- **删除第 5 版（D12）**：AI协作页与导航入口、邀请与账号授权界面；跨账号连接（配对、连接密钥、连接存储、连接 RPC）；按连接的授权、信任与额度；同账号设备开放（`sameAccountDevices`）与主机侧代理目录；经中继列出与委派远程助理（`remote:` / `conn:` 寻址的中继实现）及「最近 5 个任务」接回入口；中继的连接邀请、跨账号路由与 `cross_account_routing_v1`。不迁移，也不写清理旧文件的代码（RFC 兼容性一节）。

**Non-Goals:**
- 与共享助理对话、对话列表、审批与排队状态的使用者侧展示、在线状态置灰（F002-12）。
- 助理之间相互调用（F002-13）。
- 手机端、IM、CLI 主机上的共享与发现入口（RFC D11：不提供）。
- 按 Wi-Fi 限制共享（RFC 风险表列为后续）。

**Architecture:**
- **归属**：
  - `openbitfun-core` 新增 `lan-share` feature（启用 `openbitfun-services-integrations/lan-share`），新模块 `service/lan_share/`：
    - `shared_assistants.rs`：主人的共享存储（取代 `OpenAgentStore` 与 `ConnectionTrustStore`），位于 `<product home>/delegation/shared-assistants.json`。每个共享：助理（`assistant:<dir>`）、信任程度、保留天数、同时任务数、外部智能体提示是否已确认、授权列表（设备公钥、设备 id、显示名、设备名、授权时间、最近使用）、屏蔽的设备公钥。读不了时报告且不覆盖。
    - `shared_with_me.rs`：使用者收到的授权，位于 `<product home>/lan-share/shared-with-me.json`：主人设备 id 与公钥、显示名、设备名、助理 id、名称、说明、授权时间、状态（可用 / 已撤销）。
    - `host.rs`：共享主机。持有 `ShareIdentity`、`LanListener` 与 `Discovery`；按共享存储启停广播与监听；实现通道方法与推送；维护申请队列（截止时间、冷却、屏蔽）；向桌面端发出事件。
    - `browse.rs`：发现会话。浏览、连接每个兼容的对端读取目录、合并本机的申请与授权状态。
    - `tenants.rs`：租户工作空间。位置 `<product home>/delegation/tenants/<助理目录名>/<设备 id>/`，在助理工作区之外；由 (助理, 设备 id) 决定，不接受调用方提供的路径；提供创建、列出、按租户删除与「授权不存在且没有会话时删除」。
  - 产品语义（谁能申请、确认、撤销）只在 core；`services-integrations::lan_share` 仍只提供身份、发现与通道。
  - 桌面端 `src/apps/desktop/src/api/lan_share_api.rs`：Tauri 命令、启动时按共享存储启动主机、系统通知、事件转发；Web UI 只经 `LanShareAPI` 调用。
- **通道方法（能力 `assistant_share_v1`）**：
  - `catalog`（任何已握手的对端）：共享助理的 id、名称、说明，以及该对端对每个助理的状态（可申请 / 申请中及截止时间 / 已授权 / 冷却中及剩余秒数）。
  - `request {assistantId}`：立即返回 `{requestId, deadline}` 或明确的拒绝原因（不存在、冷却中、已有申请）；结果经推送 `decision {requestId, granted}` 送达；连接中断后用 `request_status {requestId}` 补查。
  - `remove {assistantId}`（已授权的对端）：使用者移除，主人删除该授权与这台设备的租户工作空间。主人不在线时，使用者端立即从列表移除并记下待发送的移除，下次连上主人时发送，送达前不丢弃。
  - 推送 `revoked {assistantId}`：主人撤销或停止共享。
  - 未授权的对端只能调用 `catalog`、`request`、`request_status`（RFC D3）。
- **目标端准入**：
  - 承载委派调用的主机把调用绑定到通道已校验的对端公钥（`__sourceShareKey`），替代 `__sourceConnectionId`；本计划中由测试注入，F002-12 由局域网主机在转发时绑定。
  - CLI `dispatch::delegation` 按共享存储检查：助理在共享中、公钥有授权、未被屏蔽；`connection_guard` 改为按 (公钥, 共享助理) 计数排队，撤销授权或停止共享时停止进行中的任务；`connection.rs` 的动词限制改为作用于带 `__sourceShareKey` 的调用；信任程度与保留天数取自该共享；任务的工作目录是该设备的租户工作空间（`__assistantWorkspace` 仍指向助理工作区，只读用于人设）；审计记录显示名、设备名与指纹短码。
  - 删除 `max_job_minutes`（第 6 版共享设置不含单回合时长）。
- **事件**：主机经 `EventEmitter` 发出 `lan_share://requests`（主人待确认队列）、`lan_share://shared`（共享与使用者变化）、`lan_share://shared_with_me`（授权到达、被撤销）；Web UI 订阅后刷新，不轮询。
- **远程场景**：
  - 新命令在 Product Operation Registry 中登记为 `Agnostic`、`ControllerLocal`、`REFUSED`：局域网身份与网络属于这台设备，不代理到对端。
  - Peer Device Mode：发现、共享按钮与确认框显示「对端模式下不可用」，不回退到本地数据；本机作为被控端时，确认框仍在本机弹出。
  - 远程控制（手机端、IM）：无入口；主人不在桌面前时确认框超时自动拒绝，使用者得到「未获同意」。
  - Detached Dispatch：协议版本与已发布动词不变；本计划只改变委派任务的准入来源。
  - 远程工作区：不涉及。
- **不做兼容**：F002 上线前，F002 的文件、格式与协议都不兼容旧开发版本：不迁移、不读取旧文件、不写清理代码。开发机上残留的第 5 版文件不影响新代码。

**Baseline:**
- 「助理」页：`src/web-ui/src/app/scenes/profile/views/NurseryGallery.tsx`（标题栏「新建助理」、`AssistantCard` 卡片的打开 / 关闭 / 设为主助理 / 删除）。
- 第 5 版主人侧：`core::service::delegation`（`OpenAgentStore`、`ConnectionTrustStore`、`ConnectionGrant`/`ConnectionLimits`、`sameAccountDevices`），桌面 `delegation_api.rs`、`connections_api.rs`，Web UI `AiCollaborationScene`、`AgentDelegationConfig`、`ConnectionsConfig`。
- 第 5 版目标侧：CLI `dispatch::{delegation, connection, connection_guard, audit}` 以连接 id 计；`services-integrations::remote_connect::connections` 与 core `connection_rpc` 提供连接；中继 `relay-service` 的 `connections.rs`、`admission.rs` 与 `cross_account_routing_v1`。
- 第 5 版使用者侧：`core::service::dispatch::remote_agents`（经中继实现 `RemoteAgentDelegationPort`）、`agents::registry::remote_agents`（`remote:` 列表与最近 5 个任务）、桌面 `remote_agent_delegation.rs`。`RemoteAgentDelegationPort` 与 `remote_agent_turn` 与传输无关，保留给 F002-12 接局域网实现。
- 保留并沿用：外部任务以助理身份运行、不改写助理（F002-07）；受阻告知与排队（F002-08）；会话保留与清理（F002-09）；`delegation_list_tasks` / `delegation_task_status` / `delegation_task_answer` 与导航中助理名下的外部任务列表。

**Exit Criteria:**
- 两台未登录账号的 Mac（arm64 与 x64，均只装 dmg）在同一局域网完成：
  1. A 共享助理 X（普通、7 天、2 个任务），卡片显示「共享中」；
  2. B 打开「发现」，10 秒内看到「A 的设备 · X」，悬停显示说明与指纹短码；
  3. B 申请，A 立即弹出确认框并倒数；不操作到 0 自动拒绝，B 显示「未获同意」，1 分钟内再次申请被拒并说明原因；
  4. 冷却后再次申请，A 允许，B 在 2 秒内看到 X 出现在助理列表并标明来源；A 的卡片显示「共享中 · 1 人在用」，管理视图列出 B；
  5. A 撤销 B，B 的卡片立即标「已撤销」；B 重新申请并获准后移除 X，A 的使用者列表中不再有 B；A 停止共享，B 的发现面板中 X 消失；
  6. A 退出应用，B 的申请立即得到「对方不在线」。
- 屏蔽、冷却、单个申请、截止时间、未授权对端只能读目录与申请、授权按公钥（同 id 不同公钥不适用）、撤销即停止进行中的任务、信任与保留取自共享设置、租户工作空间（同一设备的任务进入同一工作空间、不同设备互不可见、移除与删除数据时删除、授权不存在且无会话时删除），均有自动化测试。
- AI协作页、邀请、账号授权、连接、中继跨账号路由与 `cross_account_routing_v1`、经中继的远程助理列表与委派已从代码、生成的注册表与文档中删除；`pnpm run check:core-boundaries`、生成物检查与 i18n 审计通过。
- 手机端、IM、Peer Device Mode 下的行为如上所述，各有说明或测试证据。

## Task 1: 删除第 5 版 AI协作与中继跨账号能力

**Outcome:** 第 5 版的界面、连接、按连接授权、经中继的远程助理与中继跨账号路由全部删除；外部任务以助理身份运行、排队、保留与助理名下的外部任务列表保持可用。

**Files:**
- Delete: `src/web-ui/src/app/scenes/ai-collaboration/`，`src/web-ui/src/infrastructure/config/components/{AgentDelegationConfig,ConnectionsConfig}.tsx` 及测试，`src/web-ui/src/infrastructure/api/service-api/ConnectionsAPI.ts`
- Modify: `src/web-ui/src/app/scenes/{registry.ts,SceneViewport.tsx}`、`src/web-ui/src/app/components/NavPanel/MainNav.tsx`、`src/web-ui/src/app/components/SceneBar/types.ts`、`src/web-ui/src/app/appearance.ts`、`src/web-ui/src/app/components/NavPanel/appearance.ts`、`src/web-ui/src/infrastructure/appearance/registry/defaultAppearanceRegistry.ts`、`src/web-ui/src/app/components/NavPanel/sections/workspaces/DelegatedTaskList.tsx`、`src/web-ui/src/infrastructure/api/service-api/{DelegationAPI,SubagentAPI}.ts`、`src/web-ui/src/app/scenes/agents/{AgentsScene.tsx,agentsStore.ts}`、三种语言的 `common.json`、`settings/runtime.json`、`scenes/agents.json`、`flow-chat.json`
- Delete: `src/apps/desktop/src/api/{connections_api.rs,remote_agent_delegation.rs}`；Modify: `src/apps/desktop/src/api/{delegation_api.rs,dispatch_host.rs,remote_connect_api.rs,mod.rs}`、`src/apps/desktop/src/lib.rs`
- Delete: `src/crates/assembly/core/src/service/remote_connect/connection_rpc.rs`（及 `tests.rs`）、`src/crates/assembly/core/src/service/dispatch/remote_agents.rs`、`src/crates/assembly/core/src/agentic/agents/registry/remote_agents.rs`、`src/crates/assembly/core/src/agentic/coordination/remote_agent_turn/earlier_tasks.rs`（最近 5 个任务的接回）；Modify: `src/crates/assembly/core/src/agentic/coordination/{coordinator.rs,remote_agent_turn.rs}`、 `src/crates/assembly/core/src/service/{delegation.rs,remote_connect/mod.rs,remote_connect/account_runtime.rs,dispatch/mod.rs}`、`src/crates/assembly/core/src/agentic/agents/registry/{mod.rs,query.rs,types.rs}`、`src/crates/assembly/core/src/agentic/skill_agent_snapshot.rs`、`src/crates/assembly/core/src/agentic/tools/implementations/task/{execution.rs,tests.rs}`
- Delete: `src/crates/services/services-integrations/src/remote_connect/connections.rs`（及 `connections/tests.rs`）；Modify: `src/crates/services/services-integrations/src/remote_connect.rs`、`remote_connect/{relay_client.rs,realtime_client.rs,session_store.rs}`、`src/crates/services/services-integrations/src/remote_persistence.rs`
- Modify: `src/crates/services/relay-service/src/{admission.rs,lib.rs,db.rs,routes/api.rs,realtime/mod.rs,realtime/payloads.rs,account_transport_tests.rs}`；Delete: `src/crates/services/relay-service/src/connections.rs`（及 `connections/tests.rs`）
- Delete: `src/apps/cli/src/connections_cli.rs`；Modify: `src/apps/cli/src/{main.rs,account.rs,delegation_cli.rs,root_handlers.rs,peer_host/dispatch.rs,peer_host/mod.rs}`、`src/apps/cli/tests/cli_command_contracts/delegation_simulation.rs`
- Modify: `src/crates/contracts/runtime-ports/src/{lib.rs,remote_agent.rs}`（删除 `conn:` 目标与 `remote_target_connection`，端口保留）、`tests/remote_agent_port_contracts.rs`
- Modify: `src/crates/contracts/product-domains/src/remote_surface/table.rs` 与生成的注册表、`src/shared/interactive-capabilities/catalog.json` 与生成物、`docs/interactive-capabilities/`
- Modify: `src/crates/services/relay-service/AGENTS.md`、`src/apps/relay-server/README.md`、`src/crates/services/services-integrations/AGENTS.md`、`src/crates/assembly/core/AGENTS.md`、`src/apps/cli/AGENTS.md`、`docs/architecture/detached-task-dispatch.md`
- Modify（边界检查要求时）: `scripts/core-boundaries/`

**Change Map:**
- Web UI：删除 AI协作场景及其导航、外观注册与语言键；智能体列表不再显示 `remote:` 远程助理与最近任务；`DelegatedTaskList` 不再链接 AI协作页。
- 桌面端：删除 `connections_*` 命令与 `delegation_{grant_connection,revoke_connection,set_connection_limits,list_connection_trust,set_connection_trust,open_agent,close_agent,list_open_agents}`；保留 `delegation_{list_assistants,list_tasks,task_status,task_answer,audit_list}`；删除主机侧代理目录（F002-01）与经中继的远程助理接入。
- core：`delegation.rs` 删除 `OpenAgentStore`、`ConnectionTrustStore`、`ConnectionGrant`、`ConnectionLimits`、`DelegationRequester::{Device,Connection}` 与同账号设备开放，保留助理目录、外部任务工作目录、`DelegationTrust` 与工具限制、保留天数选项；删除 `connection_rpc` 与经中继的 `RemoteAgentDelegationPort` 实现，端口未注册时远程助理列表为空。
- 中继：删除连接邀请、跨账号路由与能力声明，保留账号与设备路由。
- 运行时端口：删除 `conn:` 寻址；`remote:` 地址形状保留给 F002-12 重新定义设备标识。

**Verification:**
- `cargo test --locked -p openbitfun-relay-service`
- `cargo test --locked -p openbitfun-services-integrations --no-default-features --features remote-connect --lib remote_connect::`
- `cargo test --locked -p openbitfun-runtime-ports --test remote_agent_port_contracts`
- `cargo test --locked -p openbitfun-core --lib service::delegation`
- `cargo test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `cargo check -p openbitfun-desktop`
- `pnpm --dir src/web-ui run test:run src/app/scenes/agents src/app/components/NavPanel/sections/workspaces`
- `pnpm run type-check:web`、`pnpm run lint:web`、`pnpm run i18n:audit`
- `pnpm run check:core-boundaries`
- `rg -n "cross_account_routing_v1|connections_create_invite|AiCollaboration" src docs/architecture docs/interactive-capabilities` 无结果（`ConnectionTrustStore|OpenAgentStore` 的检查移到 Task 2）

- [x] 第 5 版界面、连接、中继跨账号与经中继的远程助理已删除；保留的外部任务能力测试通过
  - 证据（2026-11-02）：
    - 中继 `cargo test --locked -p openbitfun-relay-service` 125 项通过；`services-integrations --features remote-connect --lib remote_connect::` 175 项通过；`runtime-ports --test remote_agent_port_contracts` 5 项通过。
    - core（按 `src/crates/assembly/core/AGENTS.md` 的 feature 组合）：`agentic::coordination::` 278、`agentic::tools::implementations::task::tests` 83、`agentic::agents::registry::` 63、`service::dispatch::` 56、`service::remote_connect::` 100、`service::delegation` 17，全部通过。
    - CLI：`--bin openbitfun dispatch::` 117 项、`dispatch_commands_parse_and_internal_worker_is_hidden`、`--test cli_command_contracts dispatch_probe_cli::` 通过。
    - 桌面：`cargo check --locked -p openbitfun-desktop --tests` 无新告警；`--lib -- remote_workspace_policy dispatch_host peer_host_invoke` 28 项通过（含每个命令恰有一行注册表）；`openbitfun-product-domains --no-default-features remote_surface` 20 项通过。
    - Web UI：`type-check:web` 通过；agents、NavPanel workspaces、dispatch 三处 33 个文件 290 项通过；`lint:web` 0 错误；`i18n:audit`、`theme:color-audit:all` 通过。
    - `capabilities:generate` 后 `capabilities:check` 通过（711 个命令）；`check:core-boundaries`、`node --test scripts/check-core-boundaries.test.mjs`（156 项）、`check:repo-hygiene` 通过。
    - `rg` 检查：`cross_account_routing_v1|connections_create_invite|AiCollaboration` 在 `src docs/architecture docs/interactive-capabilities` 中无结果。
  - 与计划的差异：
    - 保留 `agentic/agents/registry/remote_agents.rs` 与智能体页的远程助理显示：它是 `RemoteAgentDelegationPort` 的传输无关投影，F002-12/13 的局域网端口直接复用；没有注册端口时列表为空（桌面不再注册经中继的端口）。
    - `OpenAgentStore`、`ConnectionTrustStore`、`ConnectionGrant`、`ConnectionLimits`，CLI 目标端准入 `dispatch::{delegation,connection,connection_guard,audit}`，`bind_dispatch_source_connection` 与 Web UI `DelegatedTaskSource.kind` 留到 Task 2：Task 2 正是围绕共享存储改写这些文件，在这里先删需要一个用完即弃的替身。Task 1 已删去其中依赖连接记录与中继的部分（中继刷新、连接记录查询、按连接撤销）。
    - 经中继的远程任务改为只由设备主人审批：发起方不再在本机登记或转发审批，只提示「等待对方设备主人审批」（`OwnerWait`）。
    - 中继上的两账号模拟（`delegation_simulation.rs`、`scripts/dev/sim-devices.mjs`、桌面 `remote_agent_delegation/simulation_tests.rs`）只覆盖被删除的路径，一并删除；保留的 `dispatch probe` 能力检查移到 `dispatch_probe_cli.rs`，CLI 的 `axum`、`openbitfun-relay-service`、`openbitfun-services-integrations` 开发依赖随之删除。
    - `openbitfun delegation` 只保留 `assistants` 与 `audit`（RFC D11：CLI 主机不提供共享入口）。

## Task 2: 共享存储与目标端按设备公钥准入

**Outcome:** 主人的共享设置与授权保存在本机；Dispatch 委派任务只接受来自有授权的设备公钥，在该设备的租户工作空间中运行；信任程度、同时任务数与排队、保留天数、审计、撤销即停止都按共享的助理生效。

**Files:**
- Create: `src/crates/assembly/core/src/service/lan_share/mod.rs`、`src/crates/assembly/core/src/service/lan_share/shared_assistants.rs`、`src/crates/assembly/core/src/service/lan_share/tenants.rs`、`src/crates/assembly/core/src/service/lan_share/tests.rs`
- Modify: `src/crates/assembly/core/src/service/{mod.rs,delegation.rs,delegation_audit.rs}`、`src/crates/assembly/core/src/agentic/memories/startup.rs`（不参与记忆整理的目录改为租户根目录）、`src/crates/assembly/core/src/agentic/agents/prompt_builder/prompt_builder_impl.rs`（人设与用户画像的来源）、`src/crates/assembly/core/src/service/bootstrap/bootstrap_impl.rs`（租户不引导）、`src/crates/assembly/core/src/agentic/tools/implementations/{session_history_tool.rs,session_control_tool.rs}`（限定本租户）、`src/crates/assembly/core/Cargo.toml`（`lan-share` feature）
- Modify: `src/apps/cli/src/dispatch/{delegation.rs,connection.rs,connection_guard.rs,audit.rs,mod.rs,protocol.rs,store.rs,worker.rs}`
- Modify: `src/crates/services/services-core/src/dispatch_contract.rs`、`src/web-ui/src/features/dispatch/dispatch.contract.test.ts`
- Modify: `docs/architecture/detached-task-dispatch.md`

**Change Map:**
- `shared_assistants.rs`：
  - `SharedAssistantStore`：共享、修改设置、停止共享（同时删除全部授权）、授予、撤销、屏蔽、记录最近使用；校验保留天数取值、同时任务数 1–100、同一助理不重复共享；完全信任与外部智能体提示需要显式确认字段。
  - 授权以设备公钥为键，记录设备 id；同一设备 id 不同公钥视为不同设备。
- `tenants.rs`：租户工作空间的路径、创建（写入使用者的 `USER.md`）、列出、删除；`delegation.rs` 删除每个任务一个目录的 `delegation_workspaces_root`，`is_delegation_workspace_path` 改为判断租户根目录。
- CLI 准入：
  - 来源从 `__sourceConnectionId` 改为 `__sourceShareKey`（base64 公钥）；准入检查共享与授权；拒绝原因区分「未共享」「未授权」「已屏蔽」。
  - 工作目录：准入时取 (助理, 设备 id) 的租户工作空间，不存在则创建；同一设备的任务共用它，会话保存在这里。
- 提示与工具（`delegation.rs`、提示构建）：
  - 人设文件取自 `__assistantWorkspace`（助理工作区），用户画像取自租户工作空间的 `USER.md`，主人的 `USER.md` 与记忆摘要不进提示；租户工作空间不创建 `BOOTSTRAP.md`。
  - `SessionControl`（列出与读取）与 `SessionHistory` 从普通信任程度的禁用列表移出，改为限定在本租户工作空间：指向其他工作空间或会话时拒绝；`SessionMessage` 与其他写操作仍禁用。
  - 同时任务数按共享助理计（所有使用者合计），超出排队；运行中检查授权与共享，撤销或停止共享即停止。
  - 会话按保留天数到期清理后，授权不存在且没有会话的租户工作空间被删除。
  - 动词限制对带 `__sourceShareKey` 的调用生效：只能操作自己发起的任务、不能读文件或改审批策略与模型、不能回答审批。
  - 信任程度、保留天数从共享取；审计写入显示名、设备名、指纹短码。

**Verification:**
- `cargo test --locked -p openbitfun-core --features lan-share --lib service::lan_share`
- `cargo test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `pnpm --dir src/web-ui run test:run src/features/dispatch/dispatch.contract.test.ts`
- `pnpm run check:core-boundaries`
- `rg -n "ConnectionTrustStore|OpenAgentStore|sourceConnectionId|bind_dispatch_source_connection" src docs/architecture` 无结果

- [ ] 共享存储的设置、授权、撤销、屏蔽有测试；租户工作空间（同设备共用、不同设备隔离、删除时机）、人设继承与租户 `USER.md`、会话工具只作用于本租户有测试；未授权 / 已屏蔽 / 已撤销的公钥被拒绝且原因明确；排队、信任、保留、审计按共享生效；撤销与停止共享会停止进行中的任务

## Task 3: 共享主机、申请队列与发现会话

**Outcome:** 有共享时本机广播并监听；对端可以读目录、申请、补查与移除；主人的申请队列按截止时间、冷却、单个申请与屏蔽规则运行，结果立即推送；使用者的发现会话列出局域网内的共享助理及其状态，并保存收到的授权。

**Files:**
- Create: `src/crates/assembly/core/src/service/lan_share/{host.rs,requests.rs,browse.rs,shared_with_me.rs,protocol.rs}`
- Modify: `src/crates/assembly/core/src/service/lan_share/{mod.rs,tests.rs}`
- Create: `src/crates/assembly/core/tests/lan_share_flow.rs`（两个主机在回环地址上的完整流程）
- Modify: `src/crates/assembly/core/Cargo.toml`（测试目标）、`scripts/core-boundaries/`（测试目标与 feature 清单）、`src/crates/assembly/core/AGENTS.md`

**Change Map:**
- `protocol.rs`：通道方法与推送的请求 / 响应结构，能力名 `assistant_share_v1`。
- `requests.rs`：申请队列（纯逻辑，注入时钟）：每 (公钥, 助理) 一个进行中的申请；截止时间 60 秒；拒绝或超时后冷却 60 秒；屏蔽；允许即授予并写入共享存储。
- `host.rs`：
  - 按共享存储启停监听与广播；共享变化时更新。
  - 对未授权对端只开放 `catalog`、`request`、`request_status`；`remove` 需要授权。
  - 保存每个对端的连接，用于推送 `decision`、`revoked`；对端离线时，下次 `catalog` 中反映撤销。
  - 发出 `lan_share://requests`、`lan_share://shared`、`lan_share://shared_with_me` 事件。
- `browse.rs`：发现会话开启时浏览，按发现的对端连接并读取目录（钉住广播的公钥），合并 `shared_with_me` 的状态；协议版本或能力不兼容的设备标为「需要更新」；会话关闭时停止浏览并断开只为读目录建立的连接。
- `shared_with_me.rs`：使用者收到的授权；收到 `revoked` 或移除时更新；读不了时报告且不覆盖。
- 本机显示名修改写回 `ShareIdentity`，并刷新广播。

**Verification:**
- `cargo test --locked -p openbitfun-core --features lan-share --lib service::lan_share`
- `cargo test --locked -p openbitfun-core --features lan-share --test lan_share_flow`
- `pnpm run check:core-boundaries`

- [ ] 回环上的两个主机完成：发现 → 读目录 → 申请 → 超时拒绝 → 冷却拒绝 → 允许 → 推送到达 → 撤销推送 → 重新申请 → 移除 → 停止共享；未授权对端调用其他方法被拒绝；屏蔽的设备申请直接未获同意

## Task 4: 桌面端接入

**Outcome:** 桌面端启动时按共享存储启动共享主机；Web UI 经 Tauri 命令与事件使用共享、发现与确认；应用在后台时申请到达发系统通知。

**Files:**
- Create: `src/apps/desktop/src/api/lan_share_api.rs`
- Modify: `src/apps/desktop/src/api/mod.rs`、`src/apps/desktop/src/lib.rs`、`src/apps/desktop/Cargo.toml`（core `lan-share`）
- Modify: `src/crates/contracts/product-domains/src/remote_surface/table.rs` 与生成的注册表、`src/shared/interactive-capabilities/catalog.json` 与生成物、`docs/interactive-capabilities/`
- Create: `src/web-ui/src/infrastructure/api/service-api/LanShareAPI.ts`
- Modify: `src/apps/desktop/README.md`

**Change Map:**
- 命令（`snake_case`，结构化 `request`）：`lan_share_overview`、`lan_share_share`、`lan_share_update`、`lan_share_stop`、`lan_share_revoke`、`lan_share_delete_tenant`、`lan_share_requests`、`lan_share_decide`（允许 / 拒绝 / 拒绝并不再提示）、`lan_share_discover_start`、`lan_share_discover_stop`、`lan_share_discover_list`、`lan_share_request`、`lan_share_shared_with_me`、`lan_share_remove`、`lan_share_set_display_name`。
- 注册表各行：`Agnostic`、`ControllerLocal`、`REFUSED`。
- 事件经现有生产传输转发给 Web UI；申请到达且主窗口不在前台时发系统通知，点击打开确认框。
- 共享存储读不了时，共享入口显示原因，不启动主机。

**Verification:**
- `cargo test --locked -p openbitfun-desktop --lib api::lan_share_api`
- `cargo test --locked -p openbitfun-product-domains --lib remote_surface`
- `pnpm run check:core-boundaries`
- `pnpm run lint:rs:desktop`

- [ ] 命令与注册表一致；桌面端启动后有共享即广播、无共享不监听；后台时申请到达发通知

## Task 5: 主人界面：共享、管理与确认框

**Outcome:** 主人在助理卡片上共享、修改、撤销与停止共享；申请到达时弹出倒计时确认框。

**Files:**
- Create: `src/web-ui/src/app/scenes/profile/views/AssistantShareDialog.tsx`、`AssistantShareManageDialog.tsx`、对应 `.scss` 与测试
- Create: `src/web-ui/src/app/components/LanShareRequestDialog/LanShareRequestDialog.tsx`、`.scss` 与测试
- Create: `src/web-ui/src/infrastructure/lan-share/lanShareStore.ts` 与测试
- Modify: `src/web-ui/src/app/scenes/profile/views/{AssistantCard.tsx,NurseryGallery.tsx}`、应用根组件（挂载确认框）、三种语言的 `scenes/profile.json`（或该页所在的命名空间）
- Modify（主题审计要求时）: 对应组件的外观文件

**Change Map:**
- `AssistantCard`：操作区增加「共享」；共享中显示角标「共享中 · N 人在用」，点击打开管理视图；Peer Device Mode 下禁用并说明。
- 共享设置：信任程度（一句话说明区别；选完全信任时弹出风险确认）、保留天数、同时任务数；助理配置了外部智能体时显示一次额度提示并要求确认。
- 管理视图：修改设置、使用者列表（显示名、设备名、授权时间、最近使用）与逐个撤销、「删除该使用者的数据」（确认，删除其租户工作空间）、停止共享（确认）。
- 确认框：队列逐个显示；「允许（N）」按截止时间倒数；「拒绝」「拒绝并不再提示」；结果后自动关闭；截止时间到时显示已自动拒绝。
- 页面不出现「连接」「授权」「信任级别」「中继」等词（信任程度只在共享设置出现一次）。

**Verification:**
- `pnpm --dir src/web-ui run test:run src/app/scenes/profile/views src/app/components/LanShareRequestDialog src/infrastructure/lan-share`
- `pnpm run type-check:web`、`pnpm run lint:web`、`pnpm run i18n:audit`、`pnpm run theme:color-audit:all`

- [ ] 共享、修改、撤销、停止共享与确认框的倒计时、队列、屏蔽有组件测试；文案无内部术语

## Task 6: 使用者界面：发现面板与来自共享的助理

**Outcome:** 使用者在「发现」面板申请共享助理，获准后助理出现在列表中并标明来源，可以移除；被撤销时立即标出。

**Files:**
- Create: `src/web-ui/src/app/scenes/profile/views/DiscoverPanel.tsx`、`SharedAssistantCard.tsx`、对应 `.scss` 与测试
- Modify: `src/web-ui/src/app/scenes/profile/views/NurseryGallery.tsx`、`src/web-ui/src/infrastructure/lan-share/lanShareStore.ts`、三种语言的语言文件

**Change Map:**
- 「发现」按钮在「新建助理」左边；面板按设备分组，悬停显示详情与指纹短码；状态与剩余秒数；空状态的两种原因与打开系统设置入口；显示名修改。
- 申请：悬停详情与申请前各有一句「对方可以查看你和这个助理的对话」；发出后显示等待与倒计时（以主人截止时间为准）；结果轻提示；对方不在线、需要更新、冷却中各一句原因与下一步。
- 来自共享的助理：卡片标「来自 某某的设备」，已撤销时标出且不可用；移除需确认；点击显示说明与来源（对话入口在 F002-12）。
- Peer Device Mode 下「发现」显示不可用。

**Verification:**
- `pnpm --dir src/web-ui run test:run src/app/scenes/profile/views src/infrastructure/lan-share`
- `pnpm run type-check:web`、`pnpm run lint:web`、`pnpm run i18n:audit`、`pnpm run theme:color-audit:all`

- [ ] 发现、申请、各类失败原因、来自共享的助理的显示、撤销与移除有组件测试

## Task 7: 两台 Mac 真机验收

**Outcome:** 按 Exit Criteria 在两台只装 dmg、未登录账号的 Mac 上走完共享、发现、申请、超时、冷却、允许、撤销、移除、停止共享与对方不在线。

**Files:**
- Modify: `docs/superplan/plans/features/F002/11-share-and-request.md`（证据）

**Change Map:**
- 按 1.0.8-plus 的方式打 arm64 与 x64 dmg，分别安装；用 cua-driver 驱动两台 Mac 的界面，记录每一步的截图与时间。

**Verification:**
- 手工：Exit Criteria 第 1–6 条逐条记录。

- [ ] 两台 Mac 真机验收逐条通过，或记录失败与修复

## 风险

| 风险 | 缓解 |
|---|---|
| 删除面大，误删 F002-07/08/09 仍需要的能力 | Task 1 明确保留清单，并在删除后运行这些能力的测试；`RemoteAgentDelegationPort` 与 `remote_agent_turn` 保留给 F002-12 |
| F002-11 与 F002-12 之间，共享的助理能被授权但还不能对话 | 卡片点击只显示说明与来源；两个计划连续交付 |
| macOS 本地网络权限被拒绝时无法可靠检测 | 面板为空时同时给出两种原因与系统设置入口，不声称确定原因 |
| 冒充设备 | 连接钉住广播的完整公钥；授权按公钥；确认框显示指纹短码 |
| 申请刷屏 | 单个申请、1 分钟冷却、拒绝并不再提示、只在有共享时监听 |
| Windows 防火墙阻断入站 | 首次共享时提示；本计划的真机验收只覆盖 macOS，Windows 记录为未验证 |

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/10-lan-foundation.md`
- `docs/architecture/detached-task-dispatch.md`
- `src/web-ui/AGENTS.md`、`src/apps/desktop/AGENTS.md`、`src/crates/assembly/core/AGENTS.md`、`src/apps/cli/AGENTS.md`、`src/crates/services/relay-service/AGENTS.md`
