# Features

> 功能需求清单（人工维护）。每条需求一个 `## ` 小节，编号 `F001`、`F002` … 顺序递增、不复用；linked worktree 中自动编号会追加分支限定，如 `F001@branch-slug`。
>
> 录入方式：对 AI 说“新建 feature: <标题>”，或手动复制下方模板。
>
> `status`：`proposed`（默认，待人工复核）→ `accepted`（已确认、可规划）→ `done`（已交付）。仅当用户明确授权录入并继续规划、内容忠实且不存在实质歧义时，AI 才可直接记录为 `accepted`；否则保持 `proposed` 并等待复核。`accepted` 只批准规划，不批准实施。
>
> 正文只记录需求意图。能说清时用一段话概括要做什么、主要范围、验收结果和关键限制；设计决策、替代方案与风险论证留给 RFC，执行任务留给 plan。
>
> `requires_rfc: true` 为可选字段，缺失时按 `false`。人类明确要求或 AI 说明实质设计风险后可启用；需要 RFC 的 feature 必须先完成独立设计审批，再进入计划审批，具体格式由 Superplan RFC 流程管理。

<!-- 新增条目模板（把 F<NNN> 替换为下一个编号，例如 F001）：

## F<NNN>: 简短标题

- status: proposed
- created: YYYY-MM-DD

可选正文（一段话即可）：范围：...；验收：...；限制或非目标：...。
-->
