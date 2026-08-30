# Roundtable 多专家工作流模板 — 实施计划

**目标：** 在现有动态 Agent 委派编辑器中加入 Roundtable 模板，把
Specialist 的角色约束与 Native/ACP executor 的实际执行能力组合成一个可审批、
可持久化、可审计的多轮讨论工作流。

## 产品语义

- 每个讨论席位只配置一次 Specialist、executor，以及 Native executor 可选的
  model；同一配置复用于该席位的独立陈述和交叉审阅。
- 第一轮的席位任务互不依赖，可由现有调度器并行执行。
- 第二轮的每个席位任务依赖全部第一轮结果，因此能够比较其他观点并修订自己的
  结论。
- 主席任务依赖全部第二轮结果，负责保留分歧、整合共识并给出建议和下一步。
- 总目标会写入每个生成任务，确保脱离父轮次运行的子 Agent 仍收到真实议题。
- 生成后仍是普通的 `DynamicAgentWorkflowProposal`，用户可继续编辑每个任务的
  capabilities、预算、输出 schema、Specialist、model 和 executor。
- Roundtable 是有向无环任务图，不宣称提供实时群聊、共享隐藏上下文或
  Agent-to-Agent 自由发言。

## 范围

### 本次实现

1. 在动态 Agent 编辑器中加入可折叠的 Roundtable 模板配置。
2. 支持 2 或 3 个讨论席位以及 1 个主席；最多生成 7 个任务，保持在现有
   8-task 上限内。
3. 复用当前可用的 Specialist、Native model 和 Native/ACP executor 选项。
4. 生成两轮讨论和主席汇总的确定性任务 ID、指令与依赖关系。
5. 增加中英文 UI 文案、纯逻辑测试、Playwright UI 测试和用户文档。

### 非目标

- 不新增数据库表、工作流 schema 或第二套调度器。
- 不增加 `@all`、实时轮流发言、共享 ACP 隐藏 transcript 或无限讨论轮次。
- 不按 Codex、Claude、Kimi 等厂商名称硬编码席位。
- 不绕过现有 executor 可用性检查、审批、能力授权、预算或持久化。

## 机械验收

- 2 席模板生成 5 个任务；3 席模板生成 7 个任务。
- 第一轮任务无依赖；每个第二轮任务依赖全部第一轮任务；主席任务依赖全部
  第二轮任务。
- 同一席位的两轮任务具有相同 Specialist、executor 和 model 选择。
- 选择内置 Reviewer 时自动加入其安全策略要求的 `review` capability。
- ACP executor 自动清空 Native-only model；不可用 executor 不能选择。
- 生成的表单能够通过现有 proposal 校验、导出、创建、审批和运行路径。
- `cargo fmt --all -- --check`、`cargo test --workspace`、UI WASM check 和
  Playwright 测试全部通过。

## 状态

- [x] 核对最新 upstream/main 与现有动态委派接口
- [x] 冻结 Roundtable v1 产品边界和验收标准
- [x] 实现 Roundtable 配置与工作流生成器
- [x] 增加测试和用户文档
- [ ] 回灌个人分支并构建 macOS 应用
- [ ] 推送干净分支并创建 upstream PR
