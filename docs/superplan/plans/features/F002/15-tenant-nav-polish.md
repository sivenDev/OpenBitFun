---
id: "F002-15"
title: "导航中的共享助理：按设备分组精修与状态一眼可见"
type: "feature"
status: "in_progress"
summary: "主人端助理名下先列「我的」会话，再按使用者设备分组，分组头清楚可辨、行内不再重复「外部任务」、有待审批的分组置顶并带数量、每组限条；使用者端「来自共享」的在线、排队、等待审批与共享已停止一眼可见；不新增页面或导航入口。"
source: "docs/superplan/human/features.md"
created: "2026-11-04"
order: 15
depends_on: ["F002-12"]
parent: "F002"
---

# 导航中的共享助理：按设备分组精修 Plan

**需求基准：** RFC `docs/superplan/rfcs/F002.md` 第 7 版「AI协作需求（基准）」第 4、7 条；D7、D9（主人端按租户分组展示）、D11；「交互设计要点」的状态。F002-12 已交付按设备分组的数据与列表，本计划只改它们在导航中的呈现。

**Goal:** 主人在助理名下一眼分清自己的会话和每台使用者设备的会话，并立刻看到哪里在等自己审批；使用者在「来自共享」里一眼看到每个对话的状态和主人是否在线。

**Scope:**
- **主人端（助理名下）**
  - 有来自其他设备的会话时，自己的会话上方显示小标题「我的」；没有时不显示，和现在一样。
  - 每台使用者设备一个可折叠分组。分组头有两行：第一行是设备标识（显示名首字）加「来自 某某的设备」（RFC 原文）；第二行是「设备名 · N 个对话」，有进行中的对话时加「M 进行中」，否则加最近开始时间。分组头不再是一行被截断的文字。
  - 行内不再逐行显示「外部任务」，来源已由分组标题说明（基准 7）。每行用状态标识区分：待审批（橙色标签带数量）、进行中（圆点）、已结束（空心圆点加开始时间）。
  - 有待审批的分组用橙色底，角标显示待审批总数，排在其他设备分组前面并默认展开。其余分组默认收起；包含当前打开会话的分组展开。展开状态在本次运行内按（助理, 设备）记住。
  - 分组内的顺序：待审批、进行中，其余按开始时间从新到旧。展开时最多显示 3 行（当前打开的会话总会显示），其余收在「显示其余 N 个」里，点开显示全部。
  - 助理历史收起时（`waitingOnly`）行为不变：只列待审批和当前打开的会话，每行带来源。
  - 不显示使用者设备的在线状态：主人端没有这项数据（`OwnedShare.grants` 只有授权时间和最近使用时间）。
- **使用者端（「来自共享」）**
  - 助理名旁边：主人在线时显示绿点；不在线时整组置灰并显示「不在线」；已撤销时显示现有文案「已停止共享」。
  - 对话行使用和主人端相同的状态标识，文案沿用现有的：等待 某某 审批、排队中、进行中、已过期、共享已停止、失败。
  - 每个助理最多显示 3 个对话（当前打开的总会显示），其余收在「显示其余 N 个」里。
- **远程场景**：Peer Device Mode 下两个列表照旧不显示；审批照旧经权限邮箱，手机端和 IM 不受影响；不新增 Tauri 命令，不涉及 Product Operation Registry；没有持久化格式变化。

**Non-Goals:**
- 设备卡、「⇄」双向提示、⌘K 搜索设备名、助理页「使用者」标签页（设计方案二、三，未采用）。
- 修改 RFC 规定的文案「来自 某某的设备」。
- 主人端显示使用者设备的在线状态。
- 「来自 助理 X」标签（F002-13 在 `SessionsSection` 中负责）。
- 后端、Tauri 命令、手机端、IM。

**Architecture:**
- 纯 Web UI 改动。分组、排序、限条和分组默认展开规则放进一个纯函数模块，由 `DelegatedTaskList` 使用并单独测试；状态标识做成主人端和使用者端共用的小组件。
- 外部任务的读取从 `DelegatedTaskList` 中抽成一个 hook。`AssistantSessionHistory` 读取一次，用结果决定是否显示「我的」，并把同一份结果交给列表，不新增第二个定时读取。`WorkspaceItem` 中 `waitingOnly` 的用法保持自己读取。
- 新增的样式部件注册到 `sessions-section` 外观描述；颜色只用现有的语义 token，不新增颜色。

**Baseline:**
- F002-12 Task 3 已交付：主人端助理名下按 `source.deviceId` 分组，组标题「来自 某某 · 设备」加会话数与最近时间，每行写「外部任务」；使用者端「来自共享」列出每个助理与对话，状态文案齐全（等待审批、排队、进行中、已过期、共享已停止、失败）。截图中的问题都出自这一版呈现。
- F002-12 Task 5 仍在另一个会话、另一个分支上交付，`3a328ec4c` 改过 `SharedAssistantList`。本计划在独立 worktree（分支 `feat/f002-15-tenant-nav-polish`，基线 `8b159c359`）上进行，合并前要先合入那边的新提交并解决冲突。
- F002-13 Task 3 之后，主人端的外部任务列表不再列出助理调用；「来自 助理 X」由 `SessionsSection` 显示，不经过 `DelegatedTaskList`。F002-13 Task 4（未完成）会让完全信任下派生的会话进入租户工作空间，不在本计划范围内。
- 数据：`DelegatedTask.source` 有 `deviceId`、`displayName`、`deviceName`、`fingerprint`；`SharedWithMe` 有 `online` 与 `state`。主人端没有使用者设备的在线状态。

**Exit Criteria:**
1. 主人端用截图里的数据（一个自己的会话，加上来自同一台设备的 11 个外部任务，其中一个待审批）：助理下先是「我的」和自己的会话，然后是一个两行分组头「来自 zhengxiwan 的设备 / MacBook-Pro-90 · 11 个对话」；分组置顶、橙色、带角标 1 并展开，最多显示 3 行，下面是「显示其余 8 个」；任何一行都不再出现「外部任务」。
2. 两台设备都有外部任务、只有一台有待审批时，有待审批的分组在前并展开，另一组收起；打开收起分组里的会话后，该组保持展开。
3. 使用者端：主人在线、不在线、已停止共享三种状态可以一眼区分；等待审批和排队中的对话有状态标识；超过 3 个对话时显示「显示其余 N 个」。
4. Peer Device Mode 下两个列表不显示；助理历史收起时只列待审批和当前会话。
5. 下面各任务的验证命令全部通过，并在一台有外部任务的主人设备上用桌面端真机核对第 1 条（本地场景；局域网双机场景沿用 F002-12 的验收，不在本计划重做）。

## Task 1: 主人端按设备分组精修

**Outcome:** 主人端助理名下按 Scope 显示「我的」、两行分组头、状态标识、待审批置顶与角标、默认展开规则和每组限条；行内不再出现「外部任务」；收起时的 `waitingOnly` 行为不变。

**Files:**
- Create: `src/web-ui/src/app/components/NavPanel/sections/workspaces/delegatedTaskGroups.ts`
- Create: `src/web-ui/src/app/components/NavPanel/sections/workspaces/delegatedTaskGroups.test.ts`
- Create: `src/web-ui/src/app/components/NavPanel/sections/workspaces/NavTaskStatus.tsx`
- Modify: `src/web-ui/src/app/components/NavPanel/sections/workspaces/DelegatedTaskList.tsx`
- Modify: `src/web-ui/src/app/components/NavPanel/sections/workspaces/AssistantSessionHistory.tsx`
- Modify: `src/web-ui/src/app/components/NavPanel/sections/workspaces/WorkspaceListSection.scss`
- Modify: `src/web-ui/src/app/components/NavPanel/sections/sessions/appearance.ts`
- Modify: `src/web-ui/src/locales/{zh-CN,zh-TW,en-US}/common.json`
- Test: `src/web-ui/src/app/components/NavPanel/sections/workspaces/DelegatedTaskList.test.tsx`
- Test: `src/web-ui/src/app/components/NavPanel/sections/workspaces/AssistantSessionHistory.test.tsx`

**Change Map:**
- `delegatedTaskGroups.ts`：按 `source.deviceId` 分组；分组顺序（有待审批的在前，其余按最新任务）；组内顺序；限条与「其余 N 个」；默认展开规则。
- `DelegatedTaskList.tsx`：读取抽成 hook，可接收外部传入的任务；分组头、行状态、展开与「显示其余」；去掉行内的 `nav.sessions.delegated.external`。
- `AssistantSessionHistory.tsx`：读取一次外部任务，有任务时在 `SessionsSection` 上方显示「我的」。
- `NavTaskStatus.tsx`：待审批、进行中、已结束、排队、已过期、已停止、失败的状态标识，两个列表共用。
- `common.json`：新增「我的」、分组第二行、进行中计数、「显示其余 N 个」、角标的无障碍说明；`delegated.external` 不再使用时删除。

**Verification:**
- `pnpm --dir src/web-ui run test:run src/app/components/NavPanel/sections/workspaces`
- `pnpm run i18n:audit`

- [x] 主人端分组、排序、限条与默认展开的行为有测试并通过
- [x] 行内不再出现「外部任务」，「我的」只在有外部任务时显示
  - 证据（2026-11-04）：
    - `delegatedTaskGroups.test.ts`：状态判定（待审批优先）、分组顺序（有待审批的设备在前，其余按最新任务）、组内顺序（待审批、进行中、从新到旧）、默认展开（待审批或含当前会话）、限条（前 3 行加当前会话，其余计数）。
    - `DelegatedTaskList.test.tsx`：两行分组头（「来自 某某的设备」/ 设备 · N 个会话 · M 进行中）；待审批分组标记、角标的无障碍说明、默认展开，其他分组收起，点开后显示；行内只有状态，不再出现来源与「外部任务」；「显示其余 2 个」点开显示全部；手动收起的分组在重新挂载后仍收起；含当前会话的分组展开且该行越过限条仍显示；收起历史时（`waitingOnly`）行为与来源说明不变；Peer Device Mode 与非桌面端不读取。
    - `AssistantSessionHistory.test.tsx`：没有外部任务时不显示「我的」；有时显示，并把同一份任务交给列表，不再读第二次。
    - `pnpm --dir src/web-ui run test:run src/app/components/NavPanel/sections/workspaces`：15 个文件 105 项通过；`src/app/components/NavPanel` 全部 50 个文件 379 项通过；`pnpm run i18n:audit` 通过，0 个警告；改动文件 eslint 0 问题。
  - 与计划的差异：
    - 读取抽成 `useAssistantDelegatedTasks.ts`；展开与「显示其余」的记忆放在 `navListMemory.ts`，两个列表共用。放在组件文件里会触发 react-refresh 告警，所以单独成文件。
    - 分组第二行沿用现有的 `delegated.count`（zh-CN「N 个会话」），没有改成「对话」。
    - 行尾标签用短词（「待审批 2」「运行中」，已结束的显示开始时间），完整说明放在行的 tooltip 中；「运行中/失败」用共享词条。
    - 删除不再使用的 `delegated.external` 与 `delegated.waiting`。

## Task 2: 使用者端「来自共享」的状态与限条

**Outcome:** 「来自共享」中每个助理的在线、不在线、已停止共享一眼可辨；对话行使用共用的状态标识；每个助理最多显示 3 个对话，其余可以展开。

**Files:**
- Modify: `src/web-ui/src/app/components/NavPanel/sections/workspaces/SharedAssistantList.tsx`
- Modify: `src/web-ui/src/app/components/NavPanel/sections/workspaces/WorkspaceListSection.scss`
- Modify: `src/web-ui/src/locales/{zh-CN,zh-TW,en-US}/common.json`
- Test: `src/web-ui/src/app/components/NavPanel/sections/workspaces/SharedAssistantList.test.tsx`

**Verification:**
- `pnpm --dir src/web-ui run test:run src/app/components/NavPanel/sections/workspaces/SharedAssistantList.test.tsx`

- [x] 在线、不在线、已停止共享与各对话状态的显示有测试并通过
- [x] 超过 3 个对话时收起其余，当前打开的对话总会显示
  - 证据（2026-11-04）：`SharedAssistantList.test.tsx` 覆盖主人在线时的绿点；不在线时整组变暗、去掉绿点并写「不在线」；已停止共享时没有新对话入口；对话行的状态点、短标签与完整说明（无障碍名称与 tooltip，例如「等待 Ann 审批」「排队中 · Ann 同时只处理 2 个任务」）；5 个对话时显示前 3 个加当前打开的那个，「显示其余 1 个」点开后显示全部。与 Task 1 同一次运行，15 个文件 105 项通过。
  - 与计划的差异：对话行原来是两行（标题加完整状态），现在是一行（状态点、标题、短标签），和主人端一致；完整状态保留在无障碍名称与 tooltip 中。新增短词 `sharedAssistant.status.approvalShort`（三种语言）。
  - 真机反馈后的调整（2026-11-04，用户要求「能用图标的就用图标，hover 给文字提示」）：
    - 助理行只写名字，后面一个标识：在线是绿点，不在线是 `WifiOff`，已停止共享是 `Ban`；「来自 X 的设备 · 状态」放进 tooltip 与无障碍名称，去掉第二行。
    - 对话行与主人端任务行去掉状态点和行尾短词，改为行尾图标，沿用会话行 `SessionStatusIndicator` 的图标与色调（待审批 `Hand`、排队 `clock`、运行 `Loader2` 旋转、失败 `CircleAlert`，另加过期 `TimerOff`、共享已停止 `Ban`）；已结束的任务没有图标，开始时间放进 tooltip。完整说明改用 `Tooltip` 组件显示，同时是行的无障碍名称。
    - 删除不再使用的 `sharedAssistant.status.approvalShort`。NavPanel 50 个文件 379 项测试、`type-check:web`、改动目录 eslint、`i18n:audit`（0 警告）、`theme:color-audit:all` 通过。
    - 用户指出共享助理下对话的选中状态太浅：任务行的悬停与选中改用会话行的 `navigation-hover-feedback`、`nav-panel-text-row-active` 与 `navigation-selection-feedback`；不可用（变暗）的助理里，选中行的标题仍用主文字色。workspaces 目录 15 个文件 105 项测试与 `theme:color-audit:all` 通过。

## Task 3: 整体检查与真机核对

**Outcome:** 外观、主题与 Web UI 检查通过；在一台有外部任务的主人设备上，导航与 Exit Criteria 第 1 条一致。

**Files:**
- Test: `src/web-ui/src/app/components/NavPanel/sections/workspaces/`

**Verification:**
- `pnpm run check:web`
- `pnpm run theme:color-audit:all`
- 真机：`pnpm run desktop:dev`，在有外部任务的主人设备上截图核对导航；有来自共享的助理时同时核对使用者端

- [x] Web UI、外观与主题检查通过
  - 证据（2026-11-04）：`pnpm run check:web` 通过（canvas SDK 检查、外观契约与排版审计 0 违规、`tsc --noEmit`）；`pnpm run theme:color-audit:all` 通过，28 个范围全部通过，没有提高基线。新的样式部件（`sectionLabel`、`groupHeader`、`badge`、`statusMark`、`more`）已注册到 `sessions-section` 外观描述；颜色只用现有的语义 token。
  - 环境说明：新 worktree 里没有 `src/web-ui/src/generated/api`（由 Rust 的 ts-rs 测试导出，已被 gitignore），类型检查报找不到模块。这次从原工作区复制了同一提交（`8b159c359`）导出的文件，没有改动它们。
- [ ] 真机截图与 Exit Criteria 第 1 条一致，记录所用场景（本地）
  - 部分证据（2026-11-04，本地 `desktop:dev -- --isolated-data`，数据复制自 obf-local）：使用者端已核对，「来自共享」中已停止共享的助理只显示名字和 `Ban` 标识，对话行行尾是图标，hover 显示完整说明；选中行修正后清晰可见。主人端尚未在真机上看到按设备分组：本机 `openbitfun dispatch list` 有一个来自 agent002-Mac-Mini 的任务（`assistant:workspace`，已完成），应出现在「小岚」的历史里，但这次没能在后台打开小岚的历史会话。
  - 主人端真机（2026-11-04，用户打开的 obf-local 打包版，F001 分支合并后）：小岚名下「我的」之后是「来自 agent 的设备」一组，3 个会话，选中行清晰。用户指出分组头前面的首字母方块（「A」）没有必要，第二行「agent002-Mac-Mini · 3 个会话 · 时间」让排版难看。改为：分组头一行，只有展开箭头、「来自 X 的设备」和待审批角标；设备名、数量、运行数或时间、待审批数放进 tooltip 与无障碍名称；删除首字母方块及其样式。workspaces 目录 15 个文件 105 项测试、`type-check:web`、eslint、`check:web`、`theme:color-audit:all` 通过。改后的样子待合并到 F001 分支并重新打包后再看。
  - 改后真机（2026-11-04 12:33，F001 分支 dev 实例，合并 `26b6603a6`，obf-local 数据）：小岚名下「我的」标签之后是一行「› 来自 agent 的设备」，没有首字母方块和第二行；无障碍名称是「来自 agent 的设备 · agent002-Mac-Mini · 3 个会话 · 11月4日 11:52」。点开后 3 个会话各占一行。旁注：这个实例最初没有 `openbitfun` CLI，列表静默为空，在同一 `target/debug` 构建 CLI 后 30 秒内出现。未核对：窗口不在当前桌面，展开后箭头没有转向，可能是后台窗口暂停了 CSS 过渡；hover tooltip 也需要用户在前台看一眼。

## References
- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/12-use-shared-assistants.md`
- `docs/superplan/plans/features/F002/13-assistants-call-each-other.md`
