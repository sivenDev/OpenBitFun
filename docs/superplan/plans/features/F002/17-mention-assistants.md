---
id: "F002-17"
title: "用「@」点名助理：在能调用子智能体的对话里点名自己的或共享的助理，由当前智能体调用"
type: "feature"
status: "in_progress"
summary: "「@」菜单增加「助理」一类，列出当前对话能调用的助理（自己的其他助理与别人共享的助理，写名字与归属，不能调用的置灰写原因）。选中后成为输入框里的「@名字」标签；发送时把被点名的助理随消息带给后端，后端在回合开始时再核对一次并明确告诉智能体去调用它，调用沿用 F002-13 的调用记录与展示。可调用的判定由后端按对话给出，与智能体实际拿到的列表同一套规则；Peer Device Mode 转发到对端。"
source: "docs/superplan/human/features.md"
created: "2026-11-04"
order: 17
depends_on: ["F002-13", "F002-16"]
parent: "F002"
---

# 用「@」点名助理 Plan

**需求基准：** RFC `docs/superplan/rfcs/F002.md` 第 10 版：摘要第 10 版一段；目标 11；非目标中「@」相关两条；D15；交互设计要点「「@」点名助理」；分阶段交付 6（F002-17）；批准条件 13。主人 2026-11-04 确认：「@」助理的含义是让当前智能体去调用它；在所有能调用子智能体的对话里可用；包括共享的助理；作为 F002 的新阶段，在 F002-16 之后做。主人 2026-11-04 批准本计划（RFC 第 10 版与 F002-17）；是否先合入 `origin/main` 在 Task 3 前另行确认。

**现状（2026-11-04 核对代码）：**
- 「@」菜单 `src/web-ui/src/flow_chat/components/ChatContextPicker.tsx`（由 `RichTextInput.tsx` 的 `@` 触发、`ChatInput.tsx` 渲染）现有文件、会话、技能、MCP 与添加图片。文件以 `[File: path]` 文本进入消息；会话引用不进正文，作为 `userMessageMetadata.sessionReferences` 发送，由 `ConversationCoordinator::materialize_session_references_for_turn`（`coordinator.rs`）在回合开始时转成给智能体的内部提示。被点名的助理按会话引用的方式实现。
- 助理已经出现在智能体的可调用列表中（`skill_agent_snapshot.rs` 生成 “Available agent types for the AgentSpawn tool”）。调用的准入：`task/execution.rs` 的 `external_agent_parent_refusal`（精简模式、深度审查、权限上限、Ultra Swarm 不能调用）与 `ensure_assistant_admissible`（普通信任的外部任务不能调用助理、必须后台运行）；列表由 `registry/assistants.rs` 的 `assistant_category` 按 `AssistantCaller` 去掉自己。
- 前端现在拿不到「这个对话能调用哪些助理」：`list_subagents` 用默认的 `AssistantCaller`，不去掉自己，也不看模式与权限上限；`lan_share_shared_with_me` 是 `ControllerLocal`，Peer Device Mode 下给的是控制端自己的共享列表。
- 手机端（`RemoteSessionManager.sendMessage`）、`RemoteCommand::SendMessage` 与 IM 机器人都不带消息元数据，也没有「@」菜单。

**Goal:** 在能调用助理的对话里输入「@」，「助理」一类列出可调用的助理；选中后发送，当前智能体在这一回合调用被点名的助理，对话里出现与 F002-13 相同的调用卡片并收回结果；不能调用的对话里不出现这一类；被点名的助理在发送时已不能调用，智能体如实说明。

**Scope:**
- **后端：按对话列出可调用的助理**。core 提供一个函数，给定会话，按这一回合提供给智能体的同一套规则（调用子智能体的工具是否可用、`external_agent_parent_refusal`、`AssistantCaller` 与信任程度、去掉自己）返回可调用的助理：类型（`assistant:<id>` / `shared:<deviceId>/<assistantId>`）、名字、归属（我的 / 来自某设备）、是否可用与原因。不能调用助理的会话返回「不支持」与原因代码，不返回空列表冒充。智能体拿到的列表与这个函数共用判定，不各写一份。
- **命令**：新增 Tauri 命令 `list_callable_assistants`（结构化 `request: { sessionId }`），在 `remote_surface/table.rs` 登记一行：Peer Device Mode 转发到对端；CLI 对端主机处理（`cli_peer: HANDLED`，调用同一 core 函数）；远程工作区立场按「与工作区无关，在运行会话的设备上判定」。Web 端 `SubagentAPI`（或新 API 文件）包装。
- **回合开始时的核对与告知**：消息元数据新增可选字段 `assistantMentions: [{ agentType }]`。协调器在回合开始时（与会话引用同一位置）读取它，用上面的函数再核对：可调用的，生成内部提示，写明「用户在这条消息里点名要你调用 小岚（我的助理），类型 `assistant:xxx`；用 AgentSpawn 后台调用并在需要时用 AgentWait 收回结果；交给它的任务按用户的话决定」；不可调用的，写明原因并要求如实告诉用户。插话（`steer_dialog_turn`）同样处理。只是给智能体的指示，不在智能体循环里强制调用、不加字符串规则。
- **前端：「@」菜单**：`ChatContextPicker` 增加「助理」一类（来源行与搜索结果都参与），数据来自 `list_callable_assistants`；返回「不支持」或对端没有这个命令时不显示这一类。每项写名字与归属（复用 `remoteAgent.ts` 的命名与 `toolCards.taskTool` 的归属文案），不能调用的置灰写原因、不能选。
- **前端：标签与发送**：新增上下文类型（例如 `assistant-mention`），在输入框与发送后的消息里显示为「@名字」标签；正文里写「@名字」，便于旧版本对端与历史阅读；元数据带 `assistantMentions`。一条消息可以点名多个助理；同一助理重复点名只算一次。
- **文案**：`flow-chat` 命名空间 `contextPicker.assistants.*`，三种语言；外观部件登记新的菜单项与标签。

**Non-Goals:**
- 手机端、IM、CLI/TUI 的「@」点名（RFC 非目标）；这些入口照常可以用文字请求调用。
- 点名普通子智能体、外部智能体或模式；「@」不改变当前对话由谁回答。
- 改变调用本身（调用记录、卡片、审批、模型）——全部沿用 F002-13 与 F002-16。

**Architecture:**
- 「能否调用、能调用谁」只在 core 判定一次，命令与回合内提示都调用它；前端只展示。这样菜单与智能体看到的列表不会不一致，也满足 Peer Device Mode 由对端判定。
- 被点名的助理走消息元数据，与 `sessionReferences` 同一模式：前端不拼提示词，后端在回合开始时生成内部提示。
- 不改 Detached Dispatch 与局域网通道协议；新字段是可选元数据，旧版本忽略。

**Baseline:**
- 分支 `feat/f001-external-agent-delegation`，在 F002-16 之后（`d05670c3d`）。用户本地 `Cargo.toml` 的改动不属于本计划，不提交。
- 已核对（2026-11-04）：`origin/main` 与未合并的 PR 中没有「@」点名助理的改动。但本分支落后 `origin/main` 422 个提交，其中 `d55df6758`（feat(flow-chat): browse recent sessions in the context picker）重写了 `ChatContextPicker.tsx` 的分类结构（改为按来源 `SOURCE_LABELS`），并新增了会话来源与对应命令。Task 3 改的正是这个文件：建议在 Task 3 之前把 `origin/main` 合入本分支（单独一步，需主人同意），在新结构上加「助理」来源；否则在旧结构上实现，合入 main 时再改一遍。

**Exit Criteria:**
1. 本机（dev，隔离数据）：在一个项目对话（标准模式）里输入「@」，「助理」一类列出 小岚、小栖；在小栖的对话里只列出小岚，不列小栖自己；精简模式的对话里没有「助理」一类。
2. 选中小岚后输入框出现「@小岚」标签；发送「@小岚 只回复一个字：好」后，当前智能体在这一回合调用小岚，对话里出现「小岚 · 我的助理」的调用卡片并收回结果；发送后的消息里显示「@小岚」标签。一条消息点名两个助理时两者都被调用。
3. 共享（release 包，本机与 agent002）：在 agent002 的对话里「@」列出「小岚 · 来自 zhengxiwan 的设备」，选中发送后调用本机的小岚并收回结果；本机的小岚不在线时该项置灰写「不在线」；发送后、回合开始前小岚变得不能调用时，智能体回复说明原因。
4. Peer Device Mode：控制端连接对端后，菜单列出的是对端能调用的助理；对端为旧版本（没有新命令）时不出现「助理」一类，其他「@」功能照常。
5. 旧版本对端收到带 `assistantMentions` 的消息不出错，正文中的「@小岚」照常显示。

## Task 1: 后端按对话列出可调用的助理

**Outcome:** core 能对任一会话给出可调用的助理或「不支持」及原因，与智能体在这一回合拿到的列表一致；桌面端与 CLI 对端主机都提供这个命令。

**Files:**
- Modify: `src/crates/assembly/core/src/agentic/agents/registry/assistants.rs`、`agentic/tools/implementations/task/{mod.rs,execution.rs}`（抽出共用判定）及测试
- Modify: `src/apps/desktop/src/api/`（新命令）、`src/crates/contracts/product-domains/src/remote_surface/table.rs`（登记一行）与生成的能力文件
- Modify: `src/apps/cli/src/peer_host/commands/`（处理同一命令）
- Modify: `src/web-ui/src/infrastructure/api/service-api/`（包装）

**Change Map:**
- 抽出「会话能否调用助理」与「对这个会话可调用的助理列表」为 core 函数；`append_external_agents_from` 与 `external_agent_parent_refusal`/`ensure_assistant_admissible` 改为使用它，行为不变。
- 命令请求 `{ sessionId }`，响应 `{ supported, reason?, assistants: [{ agentType, name, owner: { kind: 'mine' | 'shared', deviceName? }, available, unavailableReason? }] }`。

**Verification:**
- core 最窄测试：精简模式 / 深度审查 / 权限上限 / 普通信任外部任务返回不支持；助理自己的对话不含自己；共享助理不在线、已撤销时列出且不可用；与回合内列表一致。
- `cargo +1.95.0 check -p openbitfun-desktop`、CLI 对端主机的闭包测试（`peer_host::`）、`pnpm run check:core-boundaries`。

- [x] 判定共用、命令与登记行、CLI 对端处理均有测试

**进展（2026-11-04）：** 完成。`task/execution.rs` 抽出 `parent_refusal`（`ParentRefusal`：审查 / 模式 / 权限上限 / Ultra Swarm），回合内的 `external_agent_parent_refusal` 与准入改为调用它，给智能体的原因文字不变。新文件 `task/callable.rs`：`callable_assistants_for_session` 从会话读取模式与工作区，要求模式带 `AgentSpawn`/`Task`，用同一 `parent_refusal`、`AssistantCaller` 与 `assistant_category` 给出列表；原因代码 `review | mode | permissionRestrictions | swarm | trust | unavailable`，不可用的共享助理带 `unavailableReason`（`RemoteAgentUnavailableReason`）。用户在界面里输入的回合既不是审查运行也不是外部委派任务，所以会话级判定不看回合元数据；普通信任外部任务的拒绝仍在回合内由 `ensure_assistant_admissible` 执行。命令 `list_callable_assistants`（`subagent_api.rs`）；登记行 `Agnostic, Proxied, HANDLED`；CLI 对端主机 `tools::list_callable_assistants`；能力目录把它记入 `browse-agents` 的证据并重新生成；Web `SubagentAPI.listCallableAssistants`。验证：`task::callable` 4 项（自己与共享的顺序和不可用原因、助理自己的对话不含自己、不能调用子智能体 / 精简 / Ultimate / 深度审查 / 权限上限返回不支持、响应结构）、`tools::implementations::task` 103 项、`remote_surface` 20 项、CLI `peer_host` 106 项、`cargo check -p openbitfun-desktop`、`check:core-boundaries`、web `tsc` 与 eslint 通过。

## Task 2: 回合开始时核对并告知智能体

**Outcome:** 带 `assistantMentions` 的消息在回合开始时转成给智能体的明确指示；不可调用的写明原因；插话同样生效。

**Files:**
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordinator.rs`（与 `materialize_session_references_for_turn` 并列）及测试
- Modify: `src/crates/contracts/runtime-ports/src/agent_api.rs`（如需登记元数据键）

**Change Map:**
- 读取元数据 `assistantMentions`，去重，逐个用 Task 1 的函数核对，生成一条内部提示（英文，给模型）。
- 字段缺失或格式不对时忽略并记日志，不让回合失败。

**Verification:**
- core 测试：可调用 → 提示含名字、归属与类型；不可调用 → 提示含原因；重复点名只出现一次；旧格式元数据不受影响；插话路径。

- [x] 核对、提示与插话均有测试

**进展（2026-11-04）：** 完成。`task/callable.rs`：`assistant_mentions` 读取元数据 `assistantMentions`（去空白、去重、按点名顺序；不是列表或缺少 `agentType` 的项记日志后跳过），`assistant_mention_reminder_for_turn(session_id, agent_type, metadata)` 用 Task 1 的判定按这一回合的模式再核对一次，生成英文内部提示：可调用的写名字、归属与 `subagent_type`，要求用 AgentSpawn 后台调用、需要时用 AgentWait 收回，交给它的任务按用户的话决定，引号内的名字只是标签；不能调用的写原因（不在线、版本不兼容、已撤销等，或「不是这个对话能调用的助理」），要求如实告诉用户；整个对话不能调用助理时写明原因。核对失败按「不可用」处理，不让回合失败。接入：`coordinator.rs` 在会话与工作区引用之后加一条 `InternalReminderKind::Generic` 提示（使用这一回合的 `runtime_agent_type`）；插话在 `scheduler.rs::buffer_steering` 里由 `prepare_assistant_mentions` 把提示追加到给模型的 `content`，显示内容不变。不在智能体循环里加强制或字符串规则。元数据键只在 core 使用，未登记到 `agent_api.rs`。验证：`task::callable` 7 项（新增：点名去重与跳过坏项、可调用与不可调用的提示、整个对话不支持时的提示）、新增 `steering_tells_the_model_about_assistants_it_names`；`session_reference`、`tools::implementations::task`、`steer` 共 130 项通过。

## Task 3: 「@」菜单的「助理」一类

**Outcome:** 能调用助理的对话里，「@」菜单出现「助理」一类，列出名字与归属，不能调用的置灰写原因；不支持或对端没有命令时不出现。

**Files:**
- Modify: `src/web-ui/src/flow_chat/components/ChatContextPicker.tsx`、`ChatInput.tsx`（传入回调）及测试
- Modify: `src/web-ui/src/locales/{zh-CN,zh-TW,en-US}/flow-chat.json`、对应外观描述

**Change Map:**
- 新的来源行与条目类型；打开菜单时按当前会话请求一次列表（会话切换时重新请求）；搜索时按名字过滤。
- 前提：按 Baseline 的说明，先决定是否合入 `origin/main`。

**Verification:**
- `pnpm --dir src/web-ui run test:run src/flow_chat/components`（菜单相关测试文件）
- `pnpm run type-check:web`、`pnpm run lint:web`、`pnpm run i18n:audit`、`pnpm run appearance:contract-audit`

- [x] 列出、置灰、隐藏与搜索均有测试

**进展（2026-11-04）：** 完成（主人同意后先在 `d1e23c7ed` 合入 `origin/main`，在新的按来源结构上实现）。`ChatContextPicker` 新增 `assistantSessionId` 与 `onSelectAssistant`：菜单打开时按当前会话调用一次 `SubagentAPI.listCallableAssistants`（按设备界面与会话为键，切换会话或设备时重新请求，旧结果丢弃）；返回「不支持」、空列表或请求失败（对端没有这个命令）时不出现「助理」来源，也不进搜索结果；列表为空时若停在「助理」视图会退回来源列表。条目写名字与归属（复用 `toolCards.taskTool.assistantOwnerMine/Shared`），不能接活的项 `aria-disabled` 并写原因（`contextPicker.assistants.*`，三种语言；标签的中文用共享术语 `$t(shared:agents.Claw)`），点击或回车不会选中；搜索按名字过滤。与输入框的连接（选中后插入「@名字」标签）放在 Task 4。验证：`ChatContextPickerOverlay.test.tsx` 25 项（新增 3 项：列出与置灰、不支持与请求失败时隐藏、搜索），`tsc`、eslint、`i18n:audit`、`appearance:contract-audit` 通过。

## Task 4: 标签、发送与消息显示

**Outcome:** 选中后成为「@名字」标签；发送时带 `assistantMentions`，正文写「@名字」；发送后的消息与历史里显示标签；插话同样带上。

**Files:**
- Modify: `src/web-ui/src/shared/types/context.ts`、`shared/utils/contextPrompt.ts`、`flow_chat/utils/composerPresentation.ts`、`flow_chat/components/RichTextInput.tsx`、`flow_chat/hooks/useMessageSender.ts`（及插话路径）、消息中引用标签的图标与显示，以及测试

**Verification:**
- 对应模块的单元测试（标签增删、多个点名去重、元数据形状、正文文字、历史回放显示）
- `pnpm run type-check:web`、`pnpm run lint:web`

- [ ] 标签、发送元数据、正文与显示均有测试

## Task 5: 真机验收

**Outcome:** 按 Exit Criteria 1、2 在本机 dev 构建（隔离数据）验收；3、5 在 release 包与 agent002 上验收（agent002 的对话点名本机共享的小岚，不需要重新申请授权）；4 在 Peer Device Mode 下验收，若当前环境无法建立对端连接，如实记录并以测试覆盖。

**Files:**
- Modify: 本计划（证据）

**Verification:**
- dev：`pnpm run desktop:dev -- --isolated-data`
- release：`zsh tmp/build-arm64.sh` 后两台安装；agent002 上的前台操作只在屏幕共享窗口中进行

- [ ] Exit Criteria 1–5 通过并记录证据

## 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| 菜单与智能体拿到的列表不一致 | 点名了却不能调用，或能调用却不在菜单里 | 判定只在 core 一处；Task 1 有一致性测试；回合开始时再核对一次并如实告知 |
| 智能体收到提示后没有调用 | 用户以为点名无效 | 提示写明是用户的明确要求与调用方式；真机验收覆盖；不在循环里强制调用（仓库规则） |
| 本分支落后 main，菜单文件已被重写 | 合入时冲突、返工 | Baseline 建议先合入 `origin/main`，需主人同意 |
| Peer Device Mode 下用到控制端本机数据 | 列出控制端的助理，调用却在对端失败 | 列表只来自转发到对端的命令；`lan_share_shared_with_me` 不用于此处 |

## References

- `docs/superplan/rfcs/F002.md`
- `docs/superplan/plans/features/F002/13-assistants-call-each-other.md`
- `docs/superplan/plans/features/F002/16-assistant-default-model.md`
- `d55df6758`（main 上重写的「@」菜单）
