---
id: "F002-18"
title: "共享助理的对话：导航写明来源，使用者可以删除对话"
type: "feature"
status: "complete"
summary: "使用者导航栏「来自共享」中共享的助理右侧直接写「来自 某某的设备」；使用者可以删除自己和共享助理的对话，主人那边的会话与任务记录一起删除，进行中的先停止，主人不在线或不支持时置灰并写原因。"
source: "docs/superplan/human/features.md"
created: "2026-11-05"
order: 18
depends_on: ["F002-12", "F002-15"]
parent: "F002"
---

# 共享助理的对话：来源与删除 Plan

**需求基准：** RFC `docs/superplan/rfcs/F002.md` 第 12 版：D7「删除一个对话」与导航栏来源、D3 能力声明、「交互设计要点」的共享助理的对话、「兼容性、安全与回滚」的协议、批准条件 14。

**Goal:** 使用者不用悬停就知道共享的助理来自谁，也能自己清理不再需要的对话，并且两边都不再留下这段对话。

**Scope:**
- **来源文字（使用者端）**：导航栏「来自共享」中，每个共享的助理名右侧写「来自 某某的设备」，用次要文字色；空间不够时先截断这段文字，助理名尽量完整。在线绿点、不在线与已撤销的图标照旧，「不在线」「已停止共享」仍在悬停提示里。
- **删除对话（使用者端）**：
  - 「来自共享」的对话行有与本机会话行一致的「…」菜单，其中有「删除」。
  - 已结束（完成、失败、已取消、已过期）的对话：确认「删除后，你和 某某 的设备上都不再保留这段对话」后，主人那边先删除，再删除本机的记录、展示副本与传输记录；行从列表消失，正开着时回到助理的其他对话或空白状态。
  - 进行中、排队或等待审批：「删除」置灰，写「先停止这个对话」。
  - 还没发出第一条消息的新对话：直接从本机删除，不确认，不联系主人。
  - 主人不在线：置灰写「不在线」。已撤销：置灰写「已停止共享」，移除整个助理仍走现有路径。主人不支持删除：置灰写「A 的设备需要更新才能删除对话」。
  - 主人那边找不到这个对话时，当作已删除，继续清理本机。
- **删除对话（主人端）**：
  - 新的 dispatch 动词 `delete`，只接受带共享密钥的调用；校验与 `status`/`cancel` 相同，不属于这台设备的对话与不存在的对话回答相同。
  - 只删除已结束的任务。删除任务记录、任务目录与它的调用会话，以及租户工作空间里这个对话的会话；做法与保留天数到期清理同一个任务相同，共用同一段代码。审计记录、租户工作空间里的文件和这台设备的其他对话不受影响。
  - 在 `probe` 的能力中声明删除，使用者据此判断是否可用。
  - 主人的列表在下一次刷新后不再有它；主人正开着它的只读视图时，显示现有的「这个外部任务的会话已不可用」。
- **远程场景**：
  - Peer Device Mode 下「来自共享」照旧不显示，不提供删除；新的 Tauri 命令在 Product Operation Registry 中登记为 `ControllerLocal`，与其他 `lan_share_*` 命令一致。
  - 手机端与 IM 没有共享助理入口，不受影响。
  - 主人端的 `delete` 只接受共享调用，Detached Dispatch 控制端与 CLI 对端主机不提供它。
  - 远程工作区不涉及。

**Non-Goals:**
- 会话数上限、主人端归档共享会话、主人逐个删除共享会话（RFC 第 12 版不做）。
- 删除运行中的对话时自动停止。
- 改动「你的助理」卡片上已有的来源文字。
- 本机助理的空会话删除（F003）。

**Architecture:**
- 主人端删除复用保留期清理：把 `store.rs` 中 `collect_expired_jobs` 对单个任务的加锁、隔离改名、删除任务目录与工作区、删除任务会话与调用会话这段抽成一个方法，清理与 `delete` 共用，保证两条路径删除的东西一致。非终态任务返回明确的「仍在运行」错误码，不删除任何东西。
- `share_call::admit_call` 把 `delete` 放进按设备校验所有权的分支；`run_dispatch_verb_canonical` 中没有共享密钥的 `delete` 直接拒绝。
- 能力常量放在 `services-core` 的 `dispatch_contract`，`dispatch_target_delete` 加入可绑定共享密钥的命令表；桌面端 `dispatch_host` 把它映射到 `delete` 动词。
- 使用者端在 `openbitfun-core` 的 `dispatch::shared_assistant` 中新增删除：读取本机记录，只处理共享助理的任务；经局域网 RPC 先探测能力，再调用 `dispatch_target_delete`；成功或对方回答找不到时删除本机传输记录与记录。各种不可用用 `SharedAssistantProblem` 的错误码表达，界面据此显示原因，不解析英文错误文本。
- 能否删除（主人是否声明了能力）按主人设备在本次运行内缓存，主人重新连上时重新探测；菜单打开时还不知道就去查一次。
- 桌面端新增两个命令：`lan_share_conversation_support`（查询主人能否删除）与 `lan_share_delete_conversation`（删除），按仓库规则使用结构化 `request`。
- 界面上「…」菜单与确认框沿用会话行的组件；空的新对话只调用本机清理，与现在「打开时丢弃未发送的对话」用同一段逻辑。
- 文案进入三种语言的 `common.json`；颜色只用现有语义 token。

**Baseline:**
- `SharedAssistantList.tsx`：来源文字只在助理名的 tooltip 与无障碍名称中；对话行是按钮，没有菜单。
- 主人端：`admit_call` 接受 `catalog/list/submit/probe/status/cancel/append/continue/query`，没有删除；保留期清理 `collect_expired_jobs` 已能完整删除一个终态任务，包括委派会话与调用会话。
- 使用者端：`forget_shared_assistant` 只在移除整个助理时删除本机全部记录；`dispatch_cancel` 已能把取消转发给主人。
- F002 尚未发布，RFC 规定 F002 内部格式不做兼容；本计划仍用能力声明来判断，因为开发中的两台设备经常是不同的构建。

**Exit Criteria:**
1. 使用者导航栏中共享的助理右侧写「来自 A 的设备」；导航栏变窄时先截断这段文字。
2. 批准条件 14 在两台 Mac 的 dev 构建上逐条通过：
   - 删除已完成的对话后两边列表都没有它，主人正开着的视图显示不可用；
   - 其他对话与租户文件不受影响；
   - 运行中的对话置灰写「先停止这个对话」，停止后可以删；
   - 空的新对话直接删除，主人那边没有变化；
   - 主人退出后置灰写「不在线」。
3. 主人端：别的设备的对话、不存在的对话、运行中的对话都删不掉，并且前两种回答相同；没有共享密钥的 `delete` 被拒绝；删除与保留期清理删除的内容一致。
4. 主人不声明能力时，使用者的「删除」置灰并说明需要更新，不会只删本机副本。
5. 下列各任务的验证命令全部通过。

## Task 1: 导航栏写明来源

**Outcome:** 「来自共享」中每个助理名右侧直接写「来自 某某的设备」，窄时先截断来源，图标与悬停提示照旧。

**Files:**
- Modify: `src/web-ui/src/app/components/NavPanel/sections/workspaces/SharedAssistantList.tsx`
- Modify: `src/web-ui/src/app/components/NavPanel/sections/workspaces/WorkspaceListSection.scss`
- Modify: `src/web-ui/src/app/components/NavPanel/sections/sessions/appearance.ts`
- Test: `src/web-ui/src/app/components/NavPanel/sections/workspaces/SharedAssistantList.test.tsx`

**Verification:**
- `pnpm --dir src/web-ui run test:run src/app/components/NavPanel/sections/workspaces/SharedAssistantList.test.tsx`

- [x] 来源文字在行内显示，在线、不在线、已撤销的标识与提示不变，有测试并通过
  - 证据（2026-11-05）：`SharedAssistantList.test.tsx` 断言来源文字在助理名之后、无障碍名称只说一次、不在线与已撤销时来源仍在；本地 dev A 截图显示「助理 [不在线图标] 来自 agent 的设备」。
  - 与计划的差异：没有改 `appearance.ts`。新增的两个样式类挂在已有的 `sessions-section` 行上，`appearance:contract-audit` 通过。

## Task 2: 主人端删除共享对话

**Outcome:** 主人的执行器接受共享调用的 `delete`，只删除这台设备自己的已结束对话，删除内容与保留期清理一致，并在 `probe` 中声明能力。

**Files:**
- Modify: `src/crates/services/services-core/src/dispatch_contract.rs`
- Modify: `src/apps/cli/src/dispatch/share_call.rs`
- Modify: `src/apps/cli/src/dispatch/mod.rs`
- Modify: `src/apps/cli/src/dispatch/store.rs`
- Modify: `src/apps/desktop/src/api/dispatch_host.rs`
- Modify: `src/crates/contracts/product-domains/src/remote_surface/table.rs`

**Change Map:**
- `dispatch_contract.rs`：删除能力常量；`dispatch_target_delete` 加入共享密钥绑定的命令表。
- `share_call.rs`：`delete` 走所有权校验分支；返回不暴露主人信息。
- `mod.rs`：`delete` 动词，没有共享密钥时拒绝；`probe` 声明能力。
- `store.rs`：从 `collect_expired_jobs` 抽出删除单个终态任务的方法，清理与删除共用；非终态返回明确错误。
- `dispatch_host.rs`：`dispatch_target_delete` → `delete`。
- `table.rs`：`dispatch_target_delete` 一行，CLI 对端主机不处理。

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-cli --bin openbitfun dispatch::`
- `cargo +1.95.0 test --locked -p openbitfun-services-core --lib dispatch_contract`
- `cargo +1.95.0 test --locked -p openbitfun-desktop --lib dispatch_host`
- `pnpm run check:core-boundaries`

- [x] 自己的已结束对话被删除，任务、会话与调用会话都不在，其他对话与租户文件还在
  - 证据（2026-11-05）：
    - `store.rs` 新增 `remove_terminal_job`，保留期清理改为调用它；测试 `a_conversation_deleted_on_request_goes_as_an_expired_one_would` 覆盖按请求删除与过期清理删除相同的内容。
    - `mod.rs` 测试 `a_device_deletes_only_finished_conversations_and_only_over_a_share`：已结束的对话删除后读不到，重复删除仍回答已删除。
- [x] 别的设备的、不存在的、运行中的对话删不掉，没有共享密钥的 `delete` 被拒绝，前两种回答相同
  - 证据（2026-11-05）：
    - `share_call.rs` 的 `a_device_acts_only_on_its_own_jobs` 把 `delete` 加入动词表：别的设备的、本机自己的与不存在的任务都回答 `dispatch job '<id>' does not exist`。
    - 运行中的任务回答 `DISPATCH_DELETE_ACTIVE_REFUSAL`，任务还在；没有共享密钥时拒绝。
    - 所有权在删除时、任务锁内再校验一次，不只靠共享调用的入口检查（安全审查后补充）：删除别的设备的或主人自己的任务回答 `does not exist`，任务还在。
- [x] `probe` 声明删除能力，保留期清理的现有测试仍通过
  - 证据（2026-11-05）：
    - `probe` 声明 `shared_conversation_delete_v1`；services-core `dispatch_contract` 8 项通过，其中删除能力属于可选能力，且共享密钥可以绑定 `dispatch_target_delete`。
    - CLI `dispatch::` 125+1 项与桌面端 `dispatch_host` 5 项通过，`check:core-boundaries` 通过。
  - 与计划的差异：`table.rs` 没有加 `dispatch_target_delete` 这一行。
    - 它不是 Tauri 命令，只在桌面端的 `dispatch_for_share_key` 里映射为 `delete`，不进入账号设备之间的目标命令表。
    - 登记表把所有 `dispatch_target_*` 都当作目标命令，测试 `target_command_family_matches_registry` 要求两者一致，加这一行反而会把它暴露给同账号的其他设备。
    - `outbound_controller_commands_are_not_target_commands` 断言它不是目标命令。

## Task 3: 使用者删除对话

**Outcome:** 使用者在对话菜单里删除共享对话：已结束的确认后两边删除，空的新对话只删本机，其余情况置灰并写原因。

**Files:**
- Modify: `src/crates/assembly/core/src/service/dispatch/shared_assistant.rs`
- Modify: `src/crates/assembly/core/src/service/dispatch/mod.rs`
- Modify: `src/apps/desktop/src/api/lan_share_api.rs`
- Modify: `src/apps/desktop/src/lib.rs`
- Modify: `src/crates/contracts/product-domains/src/remote_surface/table.rs`
- Modify: `src/web-ui/src/infrastructure/api/service-api/LanShareAPI.ts`
- Modify: `src/web-ui/src/features/dispatch/sharedAssistantConversation.ts`
- Modify: `src/web-ui/src/app/components/NavPanel/sections/workspaces/SharedAssistantList.tsx`
- Modify: `src/web-ui/src/locales/{zh-CN,zh-TW,en-US}/common.json`
- Test: `src/web-ui/src/app/components/NavPanel/sections/workspaces/SharedAssistantList.test.tsx`
- Test: `src/web-ui/src/features/dispatch/sharedAssistantConversation.test.ts`

**Change Map:**
- `shared_assistant.rs`：查询主人能否删除；删除共享对话（探测能力、调用、找不到当作已删、清理本机记录与传输记录）；不可用的错误码。
- `lan_share_api.rs`、`lib.rs`、`table.rs`：`lan_share_conversation_support` 与 `lan_share_delete_conversation` 两个命令及其登记。
- `LanShareAPI.ts`、`sharedAssistantConversation.ts`：调用封装；删除后移除本机任务与展示副本；空的新对话只做本机清理。
- `SharedAssistantList.tsx`：对话行的「…」菜单、置灰原因、确认框、删除中的状态与失败提示。

**Verification:**
- `cargo +1.95.0 test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,lan-share,dispatch-store,ssh-remote --lib -- shared_assistant`
- `cargo +1.95.0 test --locked -p openbitfun-desktop --lib remote_surface`
- `pnpm --dir src/web-ui run test:run src/app/components/NavPanel/sections/workspaces/SharedAssistantList.test.tsx src/features/dispatch`
- `pnpm run type-check:web`
- `pnpm run i18n:audit`

- [x] 主人声明能力时删除成功，两边记录清理；主人回答找不到时本机照样清理
  - 证据（2026-11-05）：
    - core `shared_assistant` 18 项通过：主人声明能力时调用 `dispatch_target_delete` 后清理本机传输记录与对话记录；主人回答找不到当作已删除；主人没有声明能力时回答 `needsUpdate`，不清理本机；进行中的拒绝映射为新错误码 `active`。
    - Web `sharedAssistantConversation.test.ts`：先删除主人端，再删除本机展示副本；没有加载的对话按任务移除；主人拒绝时本机什么都不动。
    - 桌面端 `remote_workspace_policy` 9 项、product-domains `remote_surface` 20 项通过。
    - `capabilities:generate` 只新增这两个命令的条目。
- [x] 主人不在线、已撤销、不支持、对话进行中时置灰并写对应原因，不只删本机副本
  - 证据（2026-11-05）：`SharedAssistantList.test.tsx` 覆盖以下情况：
    - 等待审批与排队的对话写「先停止这个对话」；
    - 主人不支持写「需要更新」；
    - 问不到主人、或主人不在线时写「不在线」，后者不去询问；
    - 已撤销写「已停止共享」；
    - 确认框写明主人名；取消时不删除；失败时提示原因。

    主人是否支持只在打开菜单时询问，每次运行每个主人问一次，问不到的下次再问。
  - 补充（真机核对后）：主人正开着被删除的对话时，视图原先停在删除前的内容。
    - 现在主人读到的任务列表里没有某个正在查看的任务时，关闭这个视图并提示「这个外部任务的会话已不可用」；保留期到期删除的任务同样处理。
    - 读取失败不算作任务已删除。
    - 改动在 `delegatedTaskReads.ts`、`delegatedTaskFollower.ts` 与 `FlowChatManager.ts`；新增 `delegatedTaskFollower.test.ts`，覆盖仍在列表中、读取失败与已删除三种情况，已删除的只关闭一次。
- [x] 空的新对话直接删除，不确认、不联系主人
  - 证据（2026-11-05）：主人不在线时，空的新对话的「删除」仍可用；点击后不弹确认、不询问主人，直接删除。还没发出第一条消息（状态 `submitting`，没有对话轮次）时才算空的新对话，第一条消息在途时按进行中处理。
  - Web 检查：`test:run src/app/components/NavPanel/sections/workspaces/SharedAssistantList.test.tsx src/features/dispatch` 14 个文件 167 项通过，`src/infrastructure` 227 个文件 2156 项通过；`tsc --noEmit` 与改动文件 eslint 通过；`i18n:audit` 0 个警告。

## Task 4: 整体检查与两台 Mac 真机验收

**Outcome:** Web UI、外观与主题检查通过；本地 dev A（使用者）与 agent002（主人）上按 Exit Criteria 第 1、2 条逐项核对，记录所用场景（局域网双机）。

**Files:**
- Test: `src/web-ui/src/app/components/NavPanel/sections/workspaces/`

**Verification:**
- `pnpm run check:web`
- `pnpm run theme:color-audit:all`
- `pnpm run fmt:rs`
- 真机：本地 dev A 与 agent002 的 dev 构建（部署新 CLI 与桌面端），按批准条件 14 逐条操作并截图

- [x] Web UI、外观与主题检查通过
  - 证据（2026-11-05）：
    - `theme:color-audit:all` 28 个范围通过；`appearance:contract-audit`、`theme:visual-contract`、`type-check:web` 通过；`fmt:rs` 已运行。
    - `check:web` 在排版审计停下，报 4 处违规，与 F003 记录的相同，都在本计划没有改动的 `DeviceWorkspaceSections.scss:74-75` 与 `SceneBar.scss:58-59` 中。审计没有放宽，它之后的各项单独运行均通过。
- [x] 批准条件 14 在两台 Mac 上逐条通过
  - 场景：局域网双机。本地 dev A（MacBook-Pro-90）是使用者；agent002 是主人，运行部署了本分支调试版桌面端与 CLI 的 `OpenBitFun-dev.app`。主人的共享保持普通信任、保留 7 天、同时 2 个任务。
  - 证据（2026-11-05）：
    - 来源：使用者导航栏写「助理 · 来自 agent 的设备」。
    - 已结束的对话「回复hi」：
      - 菜单中「删除」可用，确认框写「删除后，你和 agent 的设备上都不再保留这段对话」；
      - 确认后，使用者的列表与本机任务记录都没有它；
      - 主人的任务目录 `delegation-789a309a…` 与会话 `874a4ee4…` 都不在了，任务数从 33 变为 32；
      - 另一个对话 `delegation-0867c431…` 还在；租户文件 `IDENTITY.md`、`USER.md`、`.user.json` 的校验和不变。
    - 主人正开着的视图：
      - 第一次核对时，主人列表里的行消失了，打开的视图仍显示删除前的内容，不符合批准条件；见 Task 3 的补充。
      - 补充后，主人开着「山间邮差的故事」时由使用者删除它，16 秒内视图关闭，提示一次「这个外部任务的会话已不可用」。
    - 运行中的对话：
      - 运行中时「删除」置灰，写「先停止这个对话」；
      - 停止后，主人那边任务为 `cancelled`，「删除」可用，确认删除后两边都没有它；
      - 另一个自然结束的对话也照常删除。
    - 新建后没发消息的对话：「删除」可用，点击后立即消失，不弹确认，主人的任务数不变（31）。
    - 退出主人 app 后，10 秒内使用者显示「不在线」，「删除」置灰写「不在线」。重新打开主人 app 后恢复在线。
  - 没有在真机上核对的：已撤销与主人不支持删除的情况需要撤销授权或部署旧版本，由 Task 2、3 的测试覆盖。Peer Device Mode 中不显示共享助理列表；远程工作区与这项功能无关。

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/12-use-shared-assistants.md`
- `docs/superplan/plans/features/F002/15-tenant-nav-polish.md`
- `docs/architecture/remote-surface-contract.md`
