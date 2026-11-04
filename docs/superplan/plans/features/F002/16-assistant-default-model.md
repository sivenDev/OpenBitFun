---
id: "F002-16"
title: "助理的默认模型：每个助理自己的默认模型，用于它自己的新对话、被调用时与共享给别人时"
type: "feature"
status: "in_progress"
summary: "助理主页增加「默认模型」：选一个已配置的模型，或「跟随主力模型」（默认）。助理的一切运行——自己的新对话、被主人的其他助理调用、别人使用主人共享的它——都按「助理的默认模型 → 主力模型」取模型，不再跟随在某个对话里最后选过的模型。所选模型不可用时改用主力模型并在主页说明；调用卡片在运行前就写出具体模型名，共享调用显示对方回报的实际模型。"
source: "docs/superplan/human/features.md"
created: "2026-11-04"
order: 16
depends_on: ["F002-13"]
parent: "F002"
---

# 助理的默认模型 Plan

**需求基准：** RFC `docs/superplan/rfcs/F002.md` 第 9 版：摘要第 9 版一段；D10「模型（第 9 版）」；D14；分阶段交付 5（F002-16）；批准条件 12。主人 2026-11-04 批准 F002-16，并确认按以下建议写计划：助理的运行只取明确的设置（助理的默认模型，未设置时主力模型），不取「在某个对话里最后选过的模型」；对话模型菜单改写全局默认值的现有行为另行记录（B006），不在本计划内改。

**现状（2026-11-04 核对代码与 dev 数据）：**
- 助理没有自己的模型设置。新对话由 `LocalSessionDriver.createSession`（`src/web-ui/src/flow_chat/session-drivers/local/LocalSessionDriver.ts:75-86`）读全局 `ai.agent_model_selections.mode` 决定；调用自己的助理时 `build_session_config_for_workspace(workspace_id, None)`（`coordinator.rs` 助理分支），运行时经 `get_model_id_for_agent`（`agents/registry/resolution.rs`）同样落到 `mode`；别人使用共享的助理时，主人那边的准入 `apps/cli/src/dispatch/delegation.rs:147-149` 把模型置空，同样落到 `mode`。
- `mode` 初始是主力模型（`primary`），但在任一对话的模型菜单里选模型会把选择写回 `mode`（`rememberModeSelection`）。dev 数据中主力模型是 `gpt-6.1-sol`，`mode` 已被改成 `gpt-6.1-terra`，小岚被调用时用的是 terra。

**Goal:** 主人在助理 Y 的主页把默认模型设为 M 后，Y 的新对话、主人的其他助理调用 Y、别的设备使用主人共享的 Y 都用 M，调用卡片写 M；没有设置的助理一律用主力模型，不随对话里最后选过的模型变化；M 被删除或停用时 Y 照常可用，主页说明正在使用主力模型。

**Scope:**
- **设置与存储**：`AppConfig` 增加 `assistant_default_models: HashMap<String, String>`（`#[serde(default)]`），以助理 id（助理基目录下的目录名，即 `assistant:<id>` 中的 id，也是共享时的助理 id）为键、模型配置 id 为值。工作区 id 只在本机有意义，主人处理共享任务时手上只有助理 id，所以不用它作键。没有这一项、或该助理没有键时即「跟随主力模型」。只存单个模型，不存模型池。旧数据不迁移、不改写；模型被删除时不清除设置。
- **统一的解析**：core 提供「助理的运行模型」解析：该助理设置的模型存在且启用 → 用它；否则 → 主力模型槽位 `primary`；主力模型未配置时沿用现有的「请先配置主力模型」错误。返回模型引用与来源（助理设置 / 主力模型 / 所选模型不可用而改用主力模型），供界面说明。
- **生效位置**（都只在没有明确指定模型时使用解析结果；单个对话里用模型菜单改的模型只影响那个对话，照旧）：
  - 助理自己的新对话：Web `LocalSessionDriver.createSession` 对助理工作区不再读 `mode`，改用解析结果；后端创建会话时，若会话属于助理工作区且没有带模型（手机端、IM、CLI 主机等入口），同样使用解析结果。
  - 主人的其他助理调用它：协调器助理分支把解析结果作为会话模型，不再传 `None`。
  - 别的设备使用主人共享的它（外部任务与跨设备调用）：主人那边准入处把 `request.model` 设为解析结果；使用者带来的模型照旧拒绝。
- **界面**：
  - 助理主页增加「默认模型」：下拉列出已启用的模型，以及「跟随主力模型（当前为 某模型）」；所选模型不可用时在旁边写「所选模型已不可用，正在使用主力模型」，设置保留，可以改选。
  - 调用卡片运行前写具体模型名：本机调用写解析出的模型名（来自主力模型时写「某模型（主力模型）」）；共享调用写「由共享该助理的设备选择」，对方回报后写实际模型名。运行后仍以被调用会话实际使用的模型为准（`b33de0ce5`）。
- **共享调用回报实际模型**：主人那边的任务事件里，每一轮 `ModelRoundStarted` 已带实际模型名（`effective_model_name`），各版本都有，发起方读取共享任务状态时原样收到。发起方采用它作为调用卡片的模型；没有时保持「由共享该助理的设备选择」。不新增协议能力。（执行时核对：`modelSelected` 只在模型变化时写入，共享任务第一轮不会出现，所以不用它。）
- **远程场景**：设置只在桌面端助理主页提供；Peer Device Mode 下经现有配置读写转发到对端，用对端的模型列表；手机端与 IM 不提供设置，从那里发起的助理对话照常使用设置；CLI 主机读同一配置；远程工作区不涉及（助理在本机运行）。

**Non-Goals:**
- 对话模型菜单改写全局默认值的现有行为（记为 B006，另行处理）；其他对话模式（Standard、Cowork 等）的模型来源不变。
- 按助理选择模型池、推理强度或快速模型。
- 使用者为共享助理指定模型。
- 定时任务、IM 触发调用等以后的入口（RFC D10「以后的入口」）；它们以后创建助理会话时自然使用本计划的解析。

**Architecture:**
- 存储在 `config-contracts` 的 `AppConfig`，与 `assistant_default_sessions` 并列；读写沿用 Web 的 `configManager`（路径 `app.assistant_default_models`），不新增 Tauri 命令。若执行时发现需要新命令，按 `remote_surface/table.rs` 登记一行（参照 `lan_share_update`）。
- 解析放在 core（assembly 层），因为三个生效位置（Web 创建会话所走的后端、协调器助理分支、CLI 准入）都在 core 或以 core 为依赖；Web 只为了在创建前显示和传递模型而读取同一配置，判定规则以 core 为准（Web 侧规则用测试与 core 对齐）。
- 不改协议：共享调用的模型回报使用任务事件中每一轮已有的模型名。

**Baseline:**
- 分支 `feat/f001-external-agent-delegation`，基于 F002-13 当前提交（`d8a5aaaae` 之后）。用户本地 `Cargo.toml` 的改动不属于本计划，不提交。
- 动工前核对 `main` 与未合并的 PR 中是否已有助理模型设置或「对话模型菜单不改全局默认」的改动。

**Exit Criteria:**
1. 在小岚主页把默认模型设为 sol（与主力模型不同的配置时另选一个）：小岚的新对话、小栖调用小岚都用该模型，调用卡片运行前后都写该模型名。
2. 未设置默认模型的助理：在某个对话的模型菜单里选了 terra 之后，该助理的新对话与被调用时仍用主力模型。
3. 删除或停用所选模型：小岚照常可用并使用主力模型，主页写「所选模型已不可用，正在使用主力模型」，设置仍在；重新启用后恢复。
4. 共享：A 把 Y 的默认模型设为 M，B 使用 A 共享的 Y、以及 B 的助理调用 A 的 Y，A 那边的任务都用 M；B 的调用卡片在对方回报后写 M；对方为旧版本时写「由共享该助理的设备选择」。
5. 旧配置（没有 `assistant_default_models`）照常读取，所有助理按主力模型运行；新配置被旧版本读取时忽略这一项而不出错。

## Task 1: 设置的存储与解析

**Outcome:** 配置有 `assistant_default_models`，旧配置照常读入；core 能为任一助理解析出运行模型与来源。

**Files:**
- Modify: `src/crates/contracts/config-contracts/src/types.rs`（`AppConfig` 字段与往返测试）
- Create/Modify: core 中助理模型解析（放在 `src/crates/assembly/core/src/agentic/agents/` 下与 `local_assistant_workspace_id` 相邻的位置，执行时定具体文件）及其测试

**Change Map:**
- `AppConfig.assistant_default_models`，`#[serde(default)]`，与 `assistant_default_sessions` 一致；旧配置反序列化与新旧往返测试。
- `resolve_assistant_model(workspace_id) -> AssistantModelChoice { model_ref, source }`，`source` 区分助理设置、主力模型、所选模型不可用。

**Verification:**
- `cargo test --locked -p openbitfun-config-contracts`（执行时按 crate 的 `AGENTS.md` 取最窄命令）
- `cargo test --locked -p openbitfun-core --no-default-features --features agent-runtime,git,lan-share,dispatch-store,ssh-remote --lib -- assistant_model`

- [x] 旧配置读入、新旧往返、解析的三种来源均有测试

  证据（2026-11-04）：`AppConfig.assistant_default_models`（`#[serde(default)]`）；`assistant_default_models_preserve_legacy_config_and_round_trip`（旧配置读入、往返、旧版本读新配置忽略该项）通过。core `agents::registry::assistants` 的 `assistant_model_choice`/`assistant_model_for_workspace`/`assistant_model_for_assistant`（实现 Task 3 时把键由工作区 id 改为助理 id，`assistant_model_for_workspace` 先把工作区换成助理 id）：已启用的所选模型 → 它；未设置 → `primary`；所选模型停用或删除 → `primary` 并标 `ChosenUnavailable`，设置保留。`an_assistant_runs_on_its_own_model_else_the_primary_model` 通过。
  基线核对：`main` 上 `1a76b2ade` 改了 IM 创建助理会话的位置，不涉及助理模型；未合并 PR 中无相关改动。Task 2 在后端创建会话处统一取默认，IM 入口合并后同样覆盖。

## Task 2: 本机生效：新对话与被调用

**Outcome:** 助理自己的新对话与被主人其他助理调用时都用解析出的模型；不再受对话模型菜单最后一次选择的影响。

**Files:**
- Modify: `src/crates/assembly/core/src/agentic/coordination/coordinator.rs`（助理分支；创建会话时助理工作区的默认）
- Modify: `src/web-ui/src/flow_chat/session-drivers/local/LocalSessionDriver.ts` 及其测试
- 执行时核对：手机端与 IM 创建助理会话所走的后端入口（`remote_connect`）是否经过同一创建路径

**Change Map:**
- 协调器助理分支：`build_session_config_for_workspace(workspace_id, Some(resolved))`。
- 后端创建会话：属于助理工作区且没有带模型时使用解析结果。
- Web：助理工作区的新对话不读 `mode`，读助理设置，未设置时传主力模型引用。

**Verification:**
- core：`--lib -- agentic::coordination assistant_model`
- Web：`pnpm --dir src/web-ui run test:run src/flow_chat/session-drivers/local`
- `cargo check -p openbitfun-desktop`

- [x] 调用、新对话（Web 与后端入口）使用解析结果，对话菜单的最后选择不影响助理，均有测试

  证据（2026-11-04）：协调器助理分支以 `assistant_model_for_workspace` 的结果作为被调用会话的模型；所有新会话最终经 `create_session_with_workspace_and_creator_internal`，在快照全局默认值之前对助理工作区应用 `apply_assistant_model_default`（显式模型或模型池优先）。手机端、IM、CLI 主机经运行时端口 `create_agent_session_from_runtime_request` 走同一路径。Web `LocalSessionDriver` 对助理工作区不再读 `mode`、不带模型，交由后端决定。测试：core `a_new_assistant_session_starts_on_the_assistants_model_not_the_mode_default` 与 `agentic::coordination` 共 287 项通过；Web `session-drivers/local` 15 项通过（新增「助理的新对话交由后端按助理模型开始」）；`openbitfun-desktop` 检查通过、`tsc` 与 `eslint` 通过。

## Task 3: 共享生效

**Outcome:** 别的设备使用主人共享的助理时，主人那边按该助理的设置选模型；使用者带来的模型仍被拒绝。

**Files:**
- Modify: `src/apps/cli/src/dispatch/delegation.rs`（准入与选模型）及测试；`src/apps/cli/src/dispatch/mod.rs`（`submit`）

**Change Map:**
- 准入仍清空使用者带来的 `model` 与 `reasoning_preset`，并返回被准入的助理 id。
- `submit` 在重试判定之后、`select_ready_model` 之前，以 `delegation::owner_model` 设置 `request.model`：助理设置 → 主力模型，并解析成具体模型 id（目标端只接受具体 id）。放在重试判定之后，是为了主人改设置后，同一任务的重试不被判为冲突。主力模型也无法解析时留空，交给目标端原有的就绪默认值。

**Verification:**
- 按 `src/apps/cli/AGENTS.md` 的最窄测试命令运行 `dispatch::delegation` 相关测试

- [x] 共享任务使用主人的助理设置、使用者的模型被拒绝，均有测试
  证据（2026-11-04）：`delegation::admit` 仍清空使用者的 `model`/`reasoning_preset` 并返回助理 id；`submit` 在重试判定之后以 `owner_model` 选模型（与 `select_ready_model` 读同一份已初始化的配置）。`a_shared_assistant_runs_on_the_owners_choice_for_it`（未设置 → 主力；设置 → 该模型；其他助理不受影响；所选模型停用 → 主力；聊天里最后选的 `mode` 不起作用；无主力 → 留空）与 `tasks_of_one_device_share_its_tenant_workspace_and_drop_the_requested_model` 通过，`dispatch::` 共 116 项通过；core 两项解析测试、config 往返测试通过。

## Task 4: 界面：主页设置与卡片的模型名

**Outcome:** 主人在助理主页设置默认模型并看到当前生效的模型；所选模型不可用时有说明；调用卡片运行前写具体模型名。

**Files:**
- Modify: `src/web-ui/src/app/scenes/profile/views/AssistantConfigPage.tsx`、`AssistantConfigPage.appearance.ts`、`NurseryView.scss`（如需）及测试
- Modify: `src/web-ui/src/flow_chat/tool-cards/AgentControlToolCard.tsx`、`TaskToolDisplay.tsx` 及测试
- Modify: `src/web-ui/src/locales/{zh-CN,zh-TW,en-US}/{scenes/profile.json,flow-chat.json}`

**Change Map:**
- 主页「默认模型」：已启用模型列表 +「跟随主力模型（当前为 X）」；写 `app.assistant_default_models.<assistantId>`，选回「跟随主力模型」时删除该键；外观部件登记。
- 不可用说明：所选模型不在已启用列表中时显示。
- 卡片运行前：本机调用写解析出的模型名（主力模型时加注），替换「应用默认模型」。

**Verification:**
- `pnpm --dir src/web-ui run test:run src/app/scenes/profile src/flow_chat/tool-cards`
- `pnpm run type-check:web`、`pnpm run lint:web`、`pnpm run i18n:audit`、`pnpm run appearance:contract-audit`

- [x] 设置、清除、不可用说明、卡片运行前模型名均有测试
  证据（2026-11-04）：Web 端 `infrastructure/config/utils/assistantModel.ts` 按后端同一规则解析（助理设置 → 已启用则用它，否则主力模型），`useAssistantModel` 跟随 `ai.models`、`ai.default_models` 与 `app.assistant_default_models` 的变化；键为 `assistantShareId(rootPath)`，与后端 `assistant_id_of` 一致。主页新增「默认模型」部分（`AssistantModelSection`，外观部件 `model`/`modelNotice`）；旧主机没有该设置时不显示。两种调用卡片在运行前对本机助理写具体模型名，跟随主力模型时写「X（主力模型）」。测试：`AssistantModelSection.test.tsx`（默认跟随、保存、清除、不可用说明、保存失败、旧主机隐藏）、`assistantModel.test.ts`、`AgentControlToolCard.test.tsx`（运行前模型名）；profile 13 个文件 75 项，tool-cards 59 项通过；`type-check:web`、`lint:web`（改动文件无告警）、`i18n:audit`、`appearance:contract-audit` 通过。

## Task 5: 共享调用显示对方回报的模型

**Outcome:** 调用别人共享的助理时，卡片在对方回报后写实际模型；旧版本对方保持原文案。

**Files:**
- Modify: `src/crates/assembly/core/src/agentic/coordination/remote_agent_turn.rs`、`external_agent_turn.rs` 及测试
- Modify: `src/web-ui/src/flow_chat/utils/remoteAgent.ts`、`tool-cards/AgentControlToolCard.tsx`、`TaskToolDisplay.tsx` 及测试

**Change Map:**
- 发起方读取共享任务事件时记下对方每轮报告的模型名，之后开始的轮次以它命名（此前轮次误用对方设备 id 命名，改为空，空名不更新卡片）。
- 卡片对共享助理优先显示对方报告的模型；等于对方设备 id 的旧记录不当作模型，保持原文案。

**Verification:**
- 对应模块的最窄单元测试（新旧两种对方各一）

- [x] 有回报与无回报两种对方均有测试
  证据（2026-11-04）：调查发现共享调用的卡片此前会把主人的设备 id 当作模型名（`ChildSessionStream` 以设备 id 作为 `effective_model_name`），一并修正。core `calls_name_the_model_the_owners_device_reports_not_the_device`（只采纳任务会话本身的模型名、空名不覆盖、报告前的轮次不命名、报告后的轮次用报告的模型）通过，`remote_agent_turn`/`external_agent_turn` 共 35 项通过。Web `reportedSharedModel`（采用报告的模型、旧记录中的设备 id 不当作模型、只用于共享助理）通过，tool-cards 与 remoteAgent 共 49 个文件 654 项通过；`tsc`、`eslint` 通过。

## Task 6: 真机验收

**Outcome:** 按 Exit Criteria 1–3、5 在本机 dev 构建（隔离数据）验收；4 在 release 包与 agent002（172.17.9.12）上验收。

**Files:**
- Modify: 本计划（证据）

**Verification:**
- dev：`pnpm run desktop:dev -- --isolated-data`（隔离数据目录，不动用户的配置）
- release：`zsh tmp/build-arm64.sh` 后两台安装；agent002 上的前台操作只在屏幕共享窗口中进行

- [ ] Exit Criteria 1–5 全部通过并记录证据
  dev 证据（2026-11-04，隔离数据，`openbitfun-desktop` dev 构建；全局 `mode` 为 gpt-6.1-terra，主力模型为 gpt-6.1-sol）：
  - EC1：小岚主页设为 gpt-6.1-terra，写入 `app.assistant_default_models = {"workspace": "gpt-6.1-terra"}`；小栖调用小岚，卡片运行中与完成后都写「gpt-6.1-terra」，被调用会话 `dc877a63` 的用量记录为 gpt-6.1-terra。
  - EC2：小岚改回「跟随主力模型（当前为 gpt-6.1-sol）」后该键被删除；从主页发起的新对话用 gpt-6.1-sol 回复「好」；小栖（自身为 terra）调用小岚，被调用会话 `cf96f275` 用 gpt-6.1-sol，卡片写「gpt-6.1-sol」。
  - EC3：小岚设为 terra 后在设置中停用 terra：主页写「gpt-6.1-terra（不可用）」与「所选模型不可用，暂按主力模型 gpt-6.1-sol 运行。」，设置保留；新对话用 gpt-6.1-sol 回复「嗯」；重新启用后主页恢复「gpt-6.1-terra」。验收后已改回跟随主力模型。
  - EC5：隔离数据的配置早于该设置项，升级后照常读取，所有助理按主力模型运行；旧版本读新配置由 `assistant_default_models_preserve_legacy_config_and_round_trip` 覆盖。
  - 验收中发现并修正：主页「发起会话」的模型菜单对新助理对话已不起作用，只改全局最后选择，已移除（`d775dc424`）。
  - 待办：EC4 需 release 包与 agent002，恢复 A 在 agent002 上的访问需用户在屏幕共享中批准。

## 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| 未设置的助理从「跟随最后选择」变为「主力模型」 | 升级后已有助理换了模型 | 这是批准的行为（RFC 第 9 版 D14）；发布说明写明；主页显示当前生效的模型，主人可以一步设回 |
| Web 与 core 的解析规则不一致 | 卡片显示的模型与实际运行的不同 | 判定以 core 为准，Web 规则用同一组测试用例对齐；运行后卡片以实际模型为准 |
| 手机端、IM 创建助理会话绕过创建路径 | 这些入口仍用旧的全局默认 | Task 2 核对 `remote_connect` 入口，统一走后端默认 |
| 共享调用的事件路径不经过卡片 | 共享调用看不到实际模型 | Task 5 先核对路径；不通时保留原文案并在本计划记录差异，不改协议 |

## References

- `docs/superplan/rfcs/F002.md`
- B006：`docs/superplan/human/bugs.md`
- 相关提交：`b33de0ce5`（卡片显示实际模型）、`0f247224a`（助理的提问工具）
