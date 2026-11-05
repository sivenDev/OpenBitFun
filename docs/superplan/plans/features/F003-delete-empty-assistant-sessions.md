---
id: "F003"
title: "助理的空会话可以删除"
type: "feature"
status: "complete"
summary: "本机助理名下还没发出过任何一轮对话的会话，在会话菜单里可以直接删除，不需要确认；它是默认会话时清除默认设置；有内容的会话仍先归档再删除。"
source: "docs/superplan/human/features.md"
created: "2026-11-05"
order: 1
depends_on: []
parent: ""
---

# 助理的空会话可以删除 Plan

**Goal:** 用户点错「新对话」或不再需要刚建的会话时，可以直接删掉，不用先归档再到设置里删除。

**Scope:**
- 会话菜单中的「删除」只出现在同时满足以下条件的会话上，不满足时不出现，不置灰：
  - 属于本机助理（含主助理），即工作空间类型为助理；
  - 是顶层会话（不是子智能体或分支会话）；
  - 本机会话驱动的会话（不是 Detached Dispatch 的展示副本，也不是外部任务）；
  - 当前没有在运行；
  - 为空：没有发出过任何一轮对话，判断沿用复制会话已有的空会话判断 `isSessionEmptyForCopy`，它在对话轮次未加载时也能给出结果。
- 点击即删除，不弹确认。还在编辑中、尚未写到主机的草稿只在本机删除；已写到主机的空会话经现有的会话删除接口删除。
- 删除的是正在打开的会话时，与归档正在打开的会话一致，切换到同一个助理的其他会话或空白状态。
- 删除的会话是这个助理的默认会话时，删除成功后清除默认设置，助理历史中的「默认」标记随之消失。清除失败时提示，删除照常有效。
- 删除失败时提示原因，会话保留在列表里。
- 有内容的会话菜单不变，仍是归档，然后在「已归档会话」中删除。
- **远程场景**：
  - Peer Device Mode：删除与读写默认设置都走现有的对端转发路径，不新增命令。
  - 远程工作区：会话删除接口已按工作空间路由，本计划不新增路径。
  - 手机端、IM、Detached Dispatch 不涉及。
  - 不新增 Tauri 命令，不涉及 Product Operation Registry，没有持久化格式变化。

**Non-Goals:**
- 项目会话的删除。
- 有内容的会话直接删除，或批量删除。
- 共享助理上的对话：F002-18 负责，包括还没发出消息的新对话。
- 自动清理空会话。

**Architecture:**
- 纯 Web UI 改动：
  - 是否可删、删除动作与清除默认设置放进 `sessionRowActions.ts`，由 `SessionsSection` 的会话菜单调用；
  - 删除走 `FlowChatManager` 现有的删除入口，由各会话驱动处理，本机驱动区分草稿与已写入主机的会话；
  - 默认会话读写沿用 `assistantSessionPreferences.ts`。
- `SessionsSection` 需要知道当前列表属于助理：由已有的工作空间信息判断，不新增调用方参数；没有工作空间信息时不显示「删除」。
- 文案进入三种语言的 `common.json`，沿用已有的会话删除失败文案。

**Baseline:**
- 会话菜单现在有导出、重命名、复制、在定时任务中打开、归档等，没有删除；只有已归档的会话能在「设置 → 已归档会话」中删除。
- `FlowChatStore.deleteSession` 对编辑中的草稿只做本机删除，其余调用 `agentAPI.deleteSession`；分支会话由 `deleteSessionTreeBranch` 删除。
- 助理的默认会话保存在 `app.assistant_default_sessions`。会话删除时没有清除指向它的默认设置；`AssistantSessionHistory` 只在设置默认时校验会话是否可用。

**Exit Criteria:**
1. 在小岚名下新建一个对话，不发送任何消息：菜单里有「删除」，点击后它从列表消失，主机上也不再有这个会话，重启后不会回来。
2. 已发过消息的会话、项目会话、正在运行的会话、外部任务与共享助理的对话，菜单里都没有这项删除。
3. 把一个空会话设为小岚的默认会话后删除它：默认设置被清除，助理历史中不再显示「默认」。
4. 删除正在打开的空会话后，界面切换到小岚的其他会话或空白状态，不停在已删除的会话上。
5. 各任务的验证命令全部通过；第 1、3、4 条在本地 dev 构建上真机核对。Peer Device Mode 的路由由测试覆盖，因为它需要同一账号的两台设备。

## Task 1: 会话菜单删除空的助理会话

**Outcome:** 满足 Scope 条件的会话菜单中出现「删除」，点击后按 Scope 删除、切换并清除默认设置；其余会话菜单不变。

**Files:**
- Modify: `src/web-ui/src/app/components/NavPanel/sections/sessions/sessionRowActions.ts`
- Modify: `src/web-ui/src/app/components/NavPanel/sections/sessions/SessionsSection.tsx`
- Modify: `src/web-ui/src/locales/{zh-CN,zh-TW,en-US}/common.json`
- Test: `src/web-ui/src/app/components/NavPanel/sections/sessions/sessionRowActions.test.ts`
- Test: `src/web-ui/src/app/components/NavPanel/sections/sessions/SessionsSection.test.tsx`

**Change Map:**
- `sessionRowActions.ts`：判断会话能否直接删除；删除空会话并在它是默认会话时清除默认设置。
- `SessionsSection.tsx`：会话菜单中按条件显示「删除」，删除中禁用，失败时提示。

**Verification:**
- `pnpm --dir src/web-ui run test:run src/app/components/NavPanel/sections/sessions`
- `pnpm run type-check:web`
- `pnpm run i18n:audit`

- [x] 可删条件（助理、顶层、本机驱动、未运行、为空）与不出现的情况有测试并通过
- [x] 草稿只在本机删除、已写入主机的会话经删除接口删除，默认会话被清除，清除失败时删除仍有效
  - 证据（2026-11-05）：
    - `sessionRowActions.test.ts`：空的顶层助理会话与编辑中的草稿可以删除；有内容、非助理、子会话、运行中、有父会话、子智能体、已归档、导入的、临时会话，以及 dispatch 与外部任务驱动的会话都不可以；先删除再清除默认设置；删除失败时不清除默认设置并提示 `nav.sessions.deleteFailed`；只有清除失败时删除仍有效，提示 `nav.sessions.defaultSaveFailed`。
    - `assistantSessionPreferences.test.ts`：默认设置仍指向被删除的会话时才清除，且只清这个助理的；指向别的会话或主机没有这项设置时不写入。
    - `AssistantSessionHistory.test.tsx`：别处清除默认设置后，助理历史重新读取，「默认」与「清除默认」随之消失。
    - `pnpm --dir src/web-ui run test:run src/app/components/NavPanel`：64 个文件 510 项通过；`tsc --noEmit -p src/web-ui` 通过；改动文件 eslint 0 错误；`pnpm run i18n:audit` 通过，0 个警告。
  - 与计划的差异：
    - 草稿与已写入主机的会话由 `FlowChatStore.deleteSession` 区分，本计划直接复用，没有另写分支；单元测试覆盖调用顺序，主机路径在真机上核对。
    - 默认设置的清除放在 `assistantSessionPreferences.ts` 的 `clearAssistantDefaultSession`，`AssistantSessionHistory` 监听这项设置的变化。这样会话从任何一个列表删除，助理历史都会更新。
    - 新增 `nav.sessions.delete` 与 `nav.sessions.deleteFailed`（三种语言）；清除失败沿用 `nav.sessions.defaultSaveFailed`。

## Task 2: 整体检查与真机核对

**Outcome:** Web UI、外观与主题检查通过；本地 dev 构建上 Exit Criteria 第 1、3、4 条成立，记录所用场景（本地）。

**Files:**
- Test: `src/web-ui/src/app/components/NavPanel/sections/sessions/`

**Verification:**
- `pnpm run check:web`
- `pnpm run theme:color-audit:all`
- 真机：`pnpm run desktop:dev`（隔离数据），按 Exit Criteria 第 1、3、4 条操作并截图

- [x] Web UI、外观与主题检查通过
  - 证据（2026-11-05）：`pnpm run theme:color-audit:all` 28 个范围全部通过；`tsc --noEmit` 通过。
  - `pnpm run check:web` 未通过：排版审计报 4 处违规，都在本计划没有改动的文件里：
    - `NavPanel/sections/devices/DeviceWorkspaceSections.scss:74-75`
    - `SceneBar/SceneBar.scss:58-59`（`1b3f3e9cf`、`7d3540286`）

    它们是本分支此前的提交引入的，留给负责这两个文件的改动处理，本计划不放宽审计。
- [x] 真机核对第 1、3、4 条
  - 证据（2026-11-05，本地场景，F001 分支 dev 实例 A，隔离数据，WebDriver 4445）：
    - 在小岚名下新建会话，不发送：菜单是「重命名、创建副本、导出为 Markdown、定时任务、打开会话存储目录、归档、删除」；同一列表里两个有内容的会话（「调用助理创建文件」「调用助理写雨句」）菜单没有「删除」。
    - 第 3 条：把这个空会话设为默认后，主机设置 `app.assistant_default_sessions` 指向它；从菜单删除后，这项设置变为 `{}`，行从列表消失。会话存储目录中搜不到它的 id，现有会话 `842bd05d…` 的目录还在。
    - 第 4 条：删除的是正在打开的会话，主区回到空白欢迎页，没有停在已删除的会话上（截图 `f003-after-default-delete.png`，留在本地，没有入库）。
    - 第 1 条：再新建一个空会话（未设默认、未写入主机的草稿）并删除，行立即消失；重新加载窗口后两个被删除的会话都没有回来，列表中有内容的会话菜单都没有「删除」。
    - 远程工作区与 Peer Device Mode 没有真机核对：删除与默认设置都走现有接口，本计划没有新增路径；Peer Device Mode 需要同一账号的两台设备。

## References
- `docs/superplan/human/features.md`
- `src/web-ui/src/flow_chat/utils/sessionCopy.ts`
- `src/web-ui/src/app/components/NavPanel/sections/workspaces/assistantSessionPreferences.ts`
