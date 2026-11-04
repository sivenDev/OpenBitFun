# Bugs

> 缺陷清单（人工维护）。每条缺陷一个 `## ` 小节，编号 `B001`、`B002` … 顺序递增、不复用；linked worktree 中自动编号会追加分支限定，如 `B001@branch-slug`。
>
> 录入方式：对 AI 说“新建 bug: <标题>”，或手动复制下方模板。
>
> `status`：`proposed`（默认，待人工复核）→ `accepted`（已确认、可规划）→ `done`（已修复）。仅当用户明确授权录入并继续规划、内容忠实且不存在实质歧义时，AI 才可直接记录为 `accepted`；否则保持 `proposed` 并等待复核。`accepted` 只批准规划，不批准实施。

<!-- 新增条目模板（把 B<NNN> 替换为下一个编号，例如 B001）：

## B<NNN>: 简短标题

- status: proposed
- created: YYYY-MM-DD

复现步骤：
1. ...
期望：... ／ 实际：...
-->

## B001: 接受邀请的一方要打开 AI协作 页面后，才能用到对方开放的助理

- status: proposed
- created: 2026-11-02

复现步骤：
1. A 设备创建邀请，B 设备（另一账号）接受。
2. A 核对指纹并确认，把助理开放给这个连接。
3. B 不打开 AI协作 页面，直接在对话里让助理使用 A 开放的助理。

期望：B 的助理能看到并使用 A 开放的助理。／ 实际：B 本地的连接记录一直停在 `pending`，助理认为连接不可用，找不到 A 的助理；只有 B 打开 AI协作 页面（从 relay 同步一次）后才变成 `active`。

线索：requester 侧列远程助理时只读本地 `connections.json`（`usable_connection_peers`），不向 relay 同步；只有设置页的 `overview()` 会同步。实测 255 Mac（acceptor）记录为 `pending`，本机（inviter）为 `active`。

## B002: 再次登录账号后设备没有重新连上中继，助理找不到任何开放给它的助理

- status: proposed
- created: 2026-11-02

复现步骤：
1. 255 Mac 登录账号 A，退出后登录账号 B（17:35:29），随后又登录了一次账号 B（17:36:15）。
2. 不重启 OpenBitFun，在对话里让助理使用其他账号开放的助理。

期望：每次登录成功后设备都连上中继，助理能列出开放给它的助理。／ 实际：第二次登录撤销了上一个 token，但没有用新 token 重建设备连接；中继记录该设备 `online=0`，从 17:36:15 起离线。助理列远程助理时每次都报 `this device is not connected to its account relay`（只记 debug 日志），因此一个也列不出。重启应用后才恢复（启动时会自动连接）。

线索：
- 交互式登录（`login_account_on_relay_for_generation`）会 `stop_and_clear_device_routing`，但重建连接要靠前端 `AccountPanel.initializeDevices` 调 `account_connect_devices`；从其他入口登录或面板未挂载时不会重连。
- 登出后旧 epoch 的 `relay_client` 重连循环一直用已撤销的 token 重试，日志 `Relay reconnect failed: Relay authentication rejected` 一小时内 374 次；认证被拒是永久错误，不应无限重试。

## B003: 设备没连上中继时，AI协作 页和助理都没有提示

- status: proposed
- created: 2026-11-02

复现步骤：在 B002 的状态下（设备已离线），打开 AI协作 页，或在对话里让助理使用别人开放的助理。

期望：页面明确显示「未连接到中继」，助理回复「开放给你的助理暂时不可用：本机未连接中继」。／ 实际：AI协作 页照常显示连接和「完全信任」，只写「还没有可用的助理」；助理拿不到任何远程助理，也不知道原因，于是改用 ComputerUse 子智能体去点界面找「共享助理」，两次都这样（「松饼」「小水獭」）。

线索：`RemoteAgentDirectory::targets` 在连接列表不可用时只 `log::debug!` 并返回空，没有把不可用原因带给模型或界面；违反 AGENTS.md「Degrade loudly」。

## B004: 只装了 dmg 的设备不能处理别人派来的任务，且没有任何提示

- status: proposed
- created: 2026-11-02

复现步骤：
1. 两台 Mac 都只安装 OpenBitFun dmg（1.0.8-plus），不另装 `openbitfun` CLI。
2. A 把助理开放给 B 的连接；B 的助理列出或使用 A 的助理。

期望：安装 dmg 就能使用 AI协作；缺少组件时，A 的 AI协作 页和 B 的助理都说明原因。／ 实际：A 收到 `dispatch_target_probe` 后要调用 `~/.local/bin` 或 PATH 中的 `openbitfun dispatch probe`，dmg 不包含这个 CLI（`Contents/MacOS` 只有 `openbitfun-desktop`），于是失败；A 只记一行 `Connection request`，不记失败原因；B 把 A 当作 Unreachable，因为第一次就失败，列表里一个助理都没有，模型看不到原因。

线索：`src/apps/desktop/src/api/dispatch_host.rs` 的 `discover_cli`/`invoke_cli`；`RemoteAgentDirectory::query_device` 的失败只存在缓存里，`descriptors_for` 在 `agents` 为空时不产出任何条目。

## B005: AI协作 页看不出对方开放给我的助理，文字有歧义

- status: proposed
- created: 2026-11-02

复现步骤：本机把「格式转换管家」开放给 255 的连接，在 255 打开 AI协作 页。

期望：255 能看到「对方开放给你：格式转换管家」和使用方法（在对话里让助理去用）；对方显示为可读的名字。／ 实际：
1. 连接行只写「还没有可用的助理」。它的意思是 255 自己没有助理开放给对方，但读起来像「对方没开放助理给我」，用户因此以为功能坏了。
2. 页面没有任何地方列出对方开放给本机的助理。
3. 对方显示为原始 ID `user-02d6c6a6f8bd47ae83c579da48669aaf · MacBook-Pro-90.local`，没有显示名。
4. 「设备与连接 → 手机或浏览器」把自建中继（`http://172.16.21.153:9701`）标为「官方中继」。

线索：`ConnectionsConfig.tsx` 的 `grantedAssistants` 只看本机开放的助理；`connections.noAssistants` 文案。

## B006: 在对话的模型菜单里换模型，会改掉全应用的默认模型

- status: proposed
- created: 2026-11-04

复现步骤：模型设置里主力模型为 A；在任一对话的模型菜单里选模型 B；之后新建其他对话（任何模式），或让助理被调用。

期望：在一个对话里换模型只影响这个对话；全应用的默认模型只在模型设置里改。／ 实际：选择被写回全局的 `ai.agent_model_selections.mode`（`rememberModeSelection`），之后新对话与被调用的助理都用 B，与模型设置里的主力模型不一致，用户看不出原因。

线索：`src/web-ui/src/infrastructure/config/services/modelSelections.ts` 的 `rememberModeSelection`；`LocalSessionDriver.createSession` 读 `mode`。助理的运行由 F002-16 改为只取助理设置与主力模型，不受此影响；其他模式是否保留「记住最后选择」需要另行决定。

