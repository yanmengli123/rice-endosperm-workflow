# 服务器任务闭环实施计划：产物取回与远端清理

> 设计原则：**任务属于项目，产物属于项目，服务器只负责计算。** 服务器是可随时丢弃的
> 执行环境，不是文件中心。

## 现状与缺口

生命周期前中段已闭环：`runs` 表 + `RunManager`（`src-tauri/src/run_context/`）提供
detached 执行（local/WSL/SSH-direct）、状态机、生命周期租约、重启后 reconciler 重连、
`monitor_run`/`cancel_run` 工具、`transfer_between_contexts` 传输进度、本地 harvest 与
Artifact 溯源。

断点集中在尾段：

1. **远端产物取回缺失**：SSH run 的 `output_specs` 只接受显式 `ssh://` URI 并登记为
   引用（`run_context.rs` 中显式拒绝远端 glob），没有"成功后按 glob 拉回并注册为本地
   ArtifactVersion"的路径。
2. **远端工作目录清理完全没有**：`remote_workdir`（`.wisp-science/runs/<id>/`）只记录
   从不删除，没有工具、GC 或保留策略。
3. **撤回文件无概念**：上传（run inputs staging、`transfer_between_contexts`）不留
   登记，无法回答"哪些远端文件已无人引用、可以删"。
4. **无顺序保障**：没有 `harvested_at`/`cleaned_at`，无法约束"先确认取回、再允许清理"。
5. **存储位置不可选**：远端 workdir 根（`.wisp-science/runs`）、上传目标、取回落地目录
   全部硬编码；会话首次启用服务器时用户无法声明"数据上传到哪、生成文件放哪"。

## 交付策略

按小 PR 交付，每个 PR 一个持久抽象，各自带迁移、工具、测试。PR 1→2→3→4 有依赖顺序
（存储偏好为上传与取回提供目标位置；清理必须以取回确认为前置；撤回清理复用清理执行
路径）；PR 5（完成审查 modal）依赖 PR 1 的下载注册与 PR 3 的安全删除；PR 6 是收尾
策略层，可独立。

所有测试禁止真实 SSH/网络：远端行为用现有 `RunCommandRunner` fake、临时目录和
mocked Tauri 命令覆盖。迁移同时改 `crates/wisp-store/migrations/`（新编号文件）和
`crates/wisp-store/src/lib.rs` 的幂等补列，保持向后兼容。

## PR 1：Harvest v2 —— 远端产物枚举与取回

**用户问题：** SSH 任务成功后，最终结果必须能自动回到项目本地并注册为带 checksum 的
ArtifactVersion，而不是永远以 `ssh://` 引用留在服务器上。

### 设计

- 成功收尾时（`finish_remote_run`），对 SSH run 的非 `ssh://` `output_specs` 执行远端
  收集脚本（复用 `ssh_script_command` 的 `sh -s` 通道）：在 `remote_workdir` 内按 glob
  匹配文件，输出 `相对路径\t字节数\tsha256`（`sha256sum`，缺失时 `shasum -a 256`），并把
  匹配文件按相对路径硬链/复制进 `<workdir>/harvest/`。
- 单次 `scp -r` 把 `harvest/` 拉到项目 `remote/<context-label>/<run_id>/`（工作区既有
  落地区），逐文件校验 sha256，失败则整体报错、不注册。PR 2 起落地目录改由用户
  存储偏好决定。
- 对暂存目录复用现有 `harvest_run_outputs`（`src-tauri/src/harvest.rs`），走既有的
  snapshot/reference/lineage 逻辑；`source_path` 记录远端相对路径。
- 尊重大数据规则：`residency: remote`、超过 `max_file_mb`/`max_total_mb` 的文件不下载，
  由远端脚本返回的 size/checksum 直接注册为 `ssh://<alias>/<abs path>` 外部引用
  （补上现在缺失的 checksum 和 size）。**引用不得指向 workdir**（会被 PR 3/6 清理
  而悬空）：收集脚本把留在远端的产物 `mv` 出 workdir 到远端持久产物区
  （PR 2 前默认 `~/.wisp-science/artifacts/<run_id>/`，PR 2 起为
  `<remote_data_root>/artifacts/<run_id>/`），引用指向持久区路径。
- **下载本身是可恢复任务**：harvest 拉回复用现有 `kind='file_transfer'` run 机制
  （持久记录、`progress_json` 进度、应用重启后可重试），不做一次性前台 scp。
- **海量碎文件（如 Trinity 输出十几万中间文件）——选择性传输、选择性记录**：
  数据库行数只随"被选中取回的东西"增长，与远端文件总数无关。
  - `output_specs` 就是选择器：只有 spec 命中且被取回/登记的文件才进数据库；
    中间碎文件永远不产生任何记录。
  - `OutputSpec` 增加 `bundle: bool`：命中的文件在远端 `tar -czf` 成单个归档，
    只对归档算一次 sha256，下载后注册为**一个** ArtifactVersion（kind 加
    `bundle` 语义，manifest 里记录条目数与展开总字节数），可选本地解包到落地
    目录；`run_outputs` 只落一行。
  - 非 bundle 的 glob 命中数超过上限（默认 500）→ 明确报错并提示改用 `bundle`
    或收窄 glob 到最终产物（Trinity 的正确用法是 spec 指向 `Trinity.fasta`，
    中间目录留给清理）。
  - 收集脚本输出 manifest 行数同样封顶，超限即中止，不向 SQLite/UI 灌入海量行。
- 移除 `run_context.rs` 中"SSH direct output_specs must be explicit ssh:// references"
  的拒绝分支；`ssh://` URI spec 保持原语义。
- 迁移：`runs` 增加 nullable `harvested_at`（INTEGER）。本地/WSL run 在既有 harvest 成功
  后同样写入。harvest 失败不改变 run 终态，错误记入 `last_poll_error`（沿用现状），
  `harvested_at` 保持 NULL，作为 PR 3 清理的硬前置。
- 新增 `harvest_run` 工具/Tauri 命令：对 `succeeded` 且 `harvested_at IS NULL` 的 run
  手动重试取回（自动 harvest 失败后的恢复路径，也覆盖旧数据）。

### 接口

```rust
// runs 表新列
runs.harvested_at: Option<i64>

// src-tauri/src/run_context/remote.rs
fn remote_collect_script(workdir: &str, specs: &[OutputSpec]) -> String;
fn parse_collect_manifest(stdout: &str) -> Result<Vec<RemoteOutputEntry>, String>;

// src-tauri/src/harvest.rs（签名不变，新增远端入口）
async fn harvest_remote_run_outputs(store, runner, run, specs) -> Result<Vec<HarvestedArtifact>, String>;

// OutputSpec 新字段
OutputSpec { bundle: bool /* default false */, .. }

// 工具
harvest_run { run_id }
```

### 测试

- fake runner：glob 命中多文件 → manifest 解析、checksum 校验、本地注册、
  `run_outputs`/`produced` 边、`harvested_at` 写入。
- checksum 不匹配 → 报错、不注册、`harvested_at` 为 NULL、run 仍 `succeeded`。
- 超限文件 → 注册为 `ssh://` 引用且带 size/checksum，不下载；文件被移出
  workdir，引用路径位于远端持久产物区。
- 拉回走 `file_transfer` run：中断后可重试，重试幂等不重复注册。
- `bundle: true` 命中多文件 → 单个 tar 归档、单个 ArtifactVersion、单行
  `run_outputs`；归档 checksum 校验失败不注册。
- 非 bundle glob 命中数超上限 → 报错文案包含 `bundle` 建议，不部分注册。
- `logical_key` 多文件命中仍报错（与本地一致）。
- `ssh://` URI spec 行为不回归；本地/WSL run 写入 `harvested_at`。
- legacy 库重开幂等补列。

### 验证

```bash
cargo test -p wisp-science-desktop harvest
cargo test -p wisp-store runs
cargo fmt --all -- --check
```

## PR 2：服务器存储位置偏好 —— 首次启用时选择上传与取回目录

**用户问题：** 会话内第一次启用某台服务器后，用户应能选择后续分析的默认位置：
数据上传到服务器的哪个目录、run 工作目录建在哪、生成的文件取回后放进项目哪个
目录。此后所有上传与取回都遵循该默认值，可随时在 Environment 面板修改。

### 设计

- 迁移：新表 `context_storage_prefs`，按（项目 × context）保存偏好：

```sql
CREATE TABLE IF NOT EXISTS context_storage_prefs (
    project_id          TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    context_id          TEXT NOT NULL,
    remote_data_root    TEXT NOT NULL,  -- 上传数据的远端目录
    remote_workdir_root TEXT NOT NULL,  -- run 工作目录根，默认 .wisp-science/runs
    local_results_dir   TEXT NOT NULL,  -- 取回文件的项目相对目录
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    PRIMARY KEY (project_id, context_id)
);
```

- 默认值：`remote_data_root = ~/wisp/<project-slug>/data`、
  `remote_workdir_root = .wisp-science/runs`、
  `local_results_dir = remote/<context-label>`（工作区既有落地区）。
- 触发时机：会话内首次启用该 context（`set_session_execution_context_enabled` 置
  true 或首次 `run_in_context`/`transfer_between_contexts` 命中）且该（项目 ×
  context）无已存偏好时，UI 弹出存储位置对话框，预填默认值；确认后落库，之后的
  会话不再询问。对话框加入窗口级 Escape 栈（root-owned）。Agent 侧路径（无 UI
  确认时）直接使用默认值落库，不阻塞任务。
- 校验（Store 层）：远端路径必须是 HOME 相对或绝对路径且不含 `..`；
  `local_results_dir` 必须是项目相对路径且不逃逸项目根。
- 消费方接线：
  - `transfer_between_contexts` 上传缺省目标 = `remote_data_root`；
  - run workdir = `<remote_workdir_root>/<run_id>`（替换硬编码前缀，路径校验规则
    同步适配，PR 3 的删除约束以偏好中的根为准）；
  - PR 1 harvest 落地目录 = `<local_results_dir>/<run_id>`。
- UI：Environment 右栏每个 context 增加"存储位置"编辑区；设置修改只影响后续
  run/transfer，历史记录不动。

### 接口

```rust
Store::get_context_storage_prefs(project_id, context_id) -> Option<ContextStoragePrefs>
Store::upsert_context_storage_prefs(prefs)   // 含路径校验

// Tauri 命令
get_context_storage_prefs { context_id }
set_context_storage_prefs { context_id, remote_data_root, remote_workdir_root, local_results_dir }
```

### 测试

- 无偏好首次启用 → 返回"需要确认"标记；确认落库后同项目后续会话不再触发。
- 路径校验矩阵：`..`、绝对本地路径逃逸、空值全部拒绝。
- run workdir、transfer 目标、harvest 落地目录分别读取偏好（fake runner 断言下发
  路径）；无偏好时使用默认值且行为与 PR 1 一致。
- legacy 库幂等建表；旧 run 的 `remote_workdir` 不受影响。
- UI Playwright：首次启用弹出对话框，Escape 一次只关对话框；Environment 面板可改。

### 验证

```bash
cargo test -p wisp-store storage_prefs
cargo test -p wisp-science-desktop storage_prefs
cd ui && cargo check --target wasm32-unknown-unknown
cd ../ui-tests && npm ci && npx playwright test
```

## PR 3：Run 工作区清理 —— cleanup_run_workspace

**用户问题：** 任务结束后，远端 `inputs/`、日志、supervisor 文件和中间产物应能安全
删除，且绝不能在产物取回确认之前删。

### 设计

- 迁移：`runs` 增加 nullable `cleaned_at`（INTEGER）、`cleanup_error`（TEXT）。
- 新增 `cleanup_run_workspace` 工具 + Tauri 命令。前置条件（Store 层校验，不信任
  调用方）：
  - run 处于终态（`succeeded`/`failed`/`cancelled`/`timed_out`/`lost`）；
  - `succeeded` 且 `output_specs` 非空时要求 `harvested_at IS NOT NULL`；
  - 防御性检查：不存在 materialization=External 且路径位于该 workdir 内的
    ArtifactVersion（PR 1 已保证引用移出 workdir，此处兜底拒绝）；
  - 未清理过（`cleaned_at IS NULL`），重复调用幂等返回。
- 执行：SSH 走 `sh -s` 通道 `rm -rf`；local/WSL 走对应 transport 删除（Windows 用
  原生删除，不假设 POSIX）。**路径安全**：只删除 handle 中记录的 workdir，且必须
  匹配 `<remote_workdir_root>/<run_id>` 模式（PR 2 偏好中的根，默认
  `.wisp-science/runs`），拒绝任何其他路径；不展开来自远端的字符串。
- 删除失败写 `cleanup_error` 并保留 `cleaned_at` 为 NULL，可重试；成功写 `cleaned_at`
  并清空 `cleanup_error`。
- `lost` 状态的 run（进程身份无法确认）清理前先做一次 kill-by-token 兜底，避免删除
  仍在写入的目录。
- UI：run 详情/RunMonitorCard 增加"清理服务器文件"操作与已清理状态显示（图标走
  `compose_icon()` 新 kind）；`get_run_detail`/`list_runs` DTO 带出新字段。
- Agent 提示：`monitor_run` 成功返回文案中提示可清理（不自动清理，自动化留给 PR 6）。

### 接口

```rust
runs.cleaned_at: Option<i64>
runs.cleanup_error: Option<String>

Store::mark_run_cleaned(run_id, owner) -> Result<bool>
Store::record_run_cleanup_error(run_id, error)

// 工具 / Tauri 命令
cleanup_run_workspace { run_id }
```

### 测试

- 前置条件矩阵：running 拒绝；succeeded+specs+未 harvest 拒绝；harvest 后允许；
  failed/cancelled 直接允许；存在指向 workdir 的 External 引用 → 拒绝；二次调用
  幂等。
- fake runner 断言下发的删除命令路径被约束在 `.wisp-science/runs/<id>`；恶意
  workdir（`~`、`/`、含 `..`）被拒绝。
- 删除失败 → `cleanup_error` 落库、可重试；成功 → `cleaned_at` 落库。
- Windows 本地路径删除逻辑单测（不依赖 POSIX）。
- UI Playwright：run 卡片显示清理按钮 → 点击 → 状态更新；Escape 栈规则不回归。

### 验证

```bash
cargo test -p wisp-science-desktop cleanup
cd ui && cargo check --target wasm32-unknown-unknown
cd ../ui-tests && npm ci && npx playwright test
```

## PR 4：远端 staging 登记 —— 撤回与孤儿文件清理

**用户问题：** 上传到服务器但随后被撤回、替换或不再使用的文件必须可见、可清理，
使"直接丢弃这台服务器"成为可验证的操作。

### 设计

- 迁移：新表 `remote_staging`：

```sql
CREATE TABLE IF NOT EXISTS remote_staging (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    context_id  TEXT NOT NULL,
    run_id      TEXT,              -- 所属 run；transfer 类 run 也填
    remote_path TEXT NOT NULL,     -- 远端绝对/HOME 相对路径
    source      TEXT NOT NULL,     -- 'run_input' | 'transfer'
    checksum    TEXT,
    size_bytes  INTEGER,
    created_at  INTEGER NOT NULL,
    removed_at  INTEGER
);
CREATE INDEX IF NOT EXISTS ix_remote_staging_ctx ON remote_staging(context_id, removed_at);
```

- 写入点：SSH run inputs staging（`stage_inputs` 成功后逐文件登记）、
  `transfer_between_contexts` 上传成功后登记目标路径。run workdir 内的文件在 PR 3
  清理成功时批量标记 `removed_at`。
- 新增 `list_remote_files` 工具/Tauri 命令：按 context 列出本项目登记的未移除远端
  文件，标注归属 run 与其状态，区分"活跃引用"（run 未终态或未清理）与"孤儿"
  （run 已终态且已清理/不存在，或 transfer 目标已被更新版本替代——同一
  `remote_path` 存在更晚的登记）。
- 新增 `remove_remote_files` 工具：删除指定孤儿条目对应的远端文件（复用 PR 3 的
  安全删除通道；只允许删除登记在册的路径），成功标记 `removed_at`；远端文件已
  不存在（账实漂移）视为删除成功。活跃引用拒绝删除，除非显式 `force`。
- **丢弃服务器审计**：删除/注销 SSH context 前，列出仍指向该 context 的
  External ArtifactVersion 与未移除的 staging 条目，要求用户先取回或显式确认
  放弃；确认后 context 可删，引用产物标注"来源已丢弃"。
- UI：Environment 右栏每个 SSH context 增加"远端文件"视图（登记列表、孤儿标记、
  清理操作）；context 删除流程接入上述审计。

### 接口

```rust
Store::record_remote_staging(entry)
Store::list_remote_staging(context_id, include_removed)
Store::mark_remote_staging_removed(ids)

list_remote_files { context_id }
remove_remote_files { context_id, ids, force? }
```

### 测试

- run inputs staging / transfer 上传均产生登记；relay 中转不登记本地临时路径。
- 同一 `remote_path` 二次上传 → 旧条目判定为"被替换"孤儿。
- 活跃 run 引用的条目拒绝删除；`force` 可删；删除后 `removed_at` 落库。
- 远端文件已不存在 → 条目仍标记 `removed_at`，不报错。
- 未登记路径的删除请求被拒绝（防任意远端删除）。
- context 删除：存在未取回 External 引用/未移除条目 → 要求确认；确认后引用
  标注"来源已丢弃"。
- PR 3 清理联动：workdir 清理后其 inputs 条目自动标记移除。
- legacy 库幂等建表。

### 验证

```bash
cargo test -p wisp-store remote_staging
cargo test -p wisp-science-desktop remote_files
cd ui && cargo check --target wasm32-unknown-unknown
cd ../ui-tests && npx playwright test
```

## PR 5：任务完成审查 modal —— 按需下载与按需删除

**用户问题：** 远程任务完成后，用户应能在一个 modal 里看到服务器上产生的文件，
自主选择下载哪些（也可以不下载）、删除哪些（也可以不删），而不是只有"全自动
harvest + 整目录清理"两个极端。

### 设计

- 原则：**按需浏览、选择性传输、选择性记录**。枚举结果是临时数据，只在 modal
  会话内存在，**永不落库**；数据库只为用户实际选择下载/删除的对象写行，行数与
  远端文件总数（Trinity 的十几万碎文件）无关。
- 新增 `list_run_workspace_files` Tauri 命令：通过 `sh -s` 通道在 `remote_workdir`
  内**按需单层枚举**（`{ run_id, path?, name_filter?, limit?, offset? }`），返回该层
  的文件与子目录（目录带 `file_count`/`total_bytes` 汇总，供用户判断），单次
  返回行数封顶（默认 500）+ 分页；过滤在远端执行，不把全量清单拉到本地。
  UI 展开哪层才枚举哪层。local/WSL 走本地遍历，同样的分页/过滤。
- 新增 `download_run_files` 命令：只传输用户显式选中的路径（文件或目录）。
  - 文件：复用 PR 1 的收集/`scp -r`/checksum 通道下载到
    `<local_results_dir>/<run_id>/`，按 `path:<relative>` logical key 注册为
    ArtifactVersion + `run_outputs`（与 harvest 同一注册路径）——一个选择一行。
  - 目录：走 PR 1 的 bundle 通道——远端 `tar -czf` 成单个归档、归档级
    checksum、注册为**一个** ArtifactVersion，可选本地解包；绝不逐文件注册。
  - 未被选中的文件不传输、不记录，等待清理。
  - 下载注册为 `kind='file_transfer'` run（与 PR 1 harvest 拉回同一机制）：
    持久、有进度、应用重启/断网后可重试；modal 关闭不中断下载。
- 新增 `delete_run_files` 命令：给定相对路径列表，逐一校验解析后仍位于
  `<remote_workdir_root>/<run_id>` 内（拒绝 `..`、绝对路径、symlink 逃逸），复用
  PR 3 的删除通道；全目录删除仍走 `cleanup_run_workspace`。
- Modal（UI）：run 进入终态后，RunMonitorCard/run 详情出现"结果与清理"操作打开
  modal；当前会话内前台监控的 run 完成时自动打开一次。内容：
  - （#897 修订）自动打开只保留给**前台 `monitor_run` 监控**的 run（AutoRun
    卡片代表的探索型命令 run 永不自动弹），且延迟到**回合结束**（会话空闲）才
    触发；触发前后端 `should_prompt_run_review` 判定是否存在未决产物问题——
    已声明 `output_specs` 但 harvest 未完成，或未声明 specs 且工作目录非空。
    关闭 modal 通过 `dismiss_run_review` 持久化（`runs.review_dismissed_at`），
    该 run 此后永不自动弹出；手动入口不受影响。
  - 懒加载文件树 + 过滤框 + 复选框（目录可整体勾选，显示文件数与总大小），
    命中 `output_specs` 的项预勾选并标注"已自动取回"状态；
  - "下载所选"（可跳过）；"删除所选 / 清理整个工作目录"（可跳过）；关闭即什么都不做。
  - modal 加入窗口级 Escape 栈（root-owned）；下载/删除进行中禁用重复提交。
- Guard 交互：modal 内用户显式发起的删除视为用户确认，允许在未 harvest 时删除
  （PR 3 的 harvested-before-clean 前置仅约束 agent/自动路径）；删除前 UI 明确
  提示未下载的文件将丢失。
- 下载后文件同步登记进 PR 4 的 `remote_staging` 视图不需要（工作目录文件由 run
  生命周期覆盖）；删除成功后刷新列表，全目录清理后 modal 显示已清理状态。

### 接口

```rust
// 临时数据，不落库
list_run_workspace_files { run_id, path?, name_filter?, limit?, offset? }
    -> Vec<{ path, kind: file | dir, size_bytes, file_count?, mtime? }>
download_run_files { run_id, paths } -> Vec<HarvestedArtifact>  // 目录走 bundle 归档
delete_run_files { run_id, paths }  // 目录 = rm -rf 该子树
```

### 测试

- fake runner：单层枚举 + 分页 + 远端过滤，返回行数封顶；枚举结果不写任何表；
  下载注册 ArtifactVersion/run_outputs 且 checksum 校验失败时不注册，行数等于
  选择数；目录下载走 bundle 归档、只注册一个 ArtifactVersion；删除命令路径约束
  （`..`/绝对路径/越界被拒）。
- 未 harvest 的 succeeded run：agent 路径清理仍被拒，modal 用户路径允许。
- 终态才允许枚举/下载/删除；running run 拒绝。
- UI Playwright：run 完成 → modal 自动打开一次；Escape 一次只关 modal 且 run 详情
  保持打开；勾选下载显示进度；不下载直接删除有丢失警告；全部跳过关闭无副作用。

### 验证

```bash
cargo test -p wisp-science-desktop run_files
cd ui && cargo check --target wasm32-unknown-unknown
cd ../ui-tests && npm ci && npx playwright test
```

## PR 6：保留策略 —— 成功任务自动清理（收尾，可选）

**用户问题：** 用户不应手动清理每个任务；成功且已取回的任务应按项目策略自动回收
远端空间。

### 设计

- 项目级设置 `run_workspace_retention_days`（默认 NULL=不自动清理，显式 opt-in）。
- 复用 `RunManager` 的后台 poller/reconciler 周期：扫描
  `succeeded && harvested_at IS NOT NULL && cleaned_at IS NULL && ended_at < now - N days`
  的 run，逐个走 PR 3 的 `cleanup_run_workspace` 路径（含全部前置校验与路径约束）。
- 独立设置 `failed_run_retention_days`（默认 NULL，建议窗口更长）：对
  `failed`/`cancelled`/`timed_out` 的 run 同样按期回收 workdir——失败任务的碎文件
  （如跑挂的 Trinity）常常最大，不能永久堆积；日志 tail 已在库中，不因清理丢失。
- 清理动作写入 run 时间线（`cleanup_error`/`cleaned_at` 已有），UI 设置页暴露开关。
- 文档：更新 `skills/remote-compute-ssh/SKILL.md` 与 control-plane spec，写明生命周期
  九段全部落地：上传 → 创建 → 后台执行 → 状态 → 日志 → 取消/重连 → 登记 → 取回 → 清理。

### 测试

- 到期 run 被自动清理、未到期/未 harvest 不动。
- `failed_run_retention_days` 只影响失败/取消/超时态，且与成功态窗口互不干扰。
- 清理失败不阻塞 poller，错误落库且下轮重试。
- 两个设置默认关闭；开启/关闭即时生效。

### 验证

```bash
cargo test -p wisp-science-desktop retention
cargo test --workspace
cargo fmt --all -- --check
```

## 风险与限制

- **远端工具依赖**：收集脚本依赖 `sha256sum`/`shasum`、`tar` 或硬链支持；沿用 SSH
  runner 既有的 preflight 模式显式探测并给出可读错误，不静默降级。
- **大量小文件（数据库记录爆炸防线）**：一切按"选择性传输、选择性记录"设计——
  枚举是按需、分页、临时的，永不落库；数据库行数只随用户/spec 的选择增长。海量
  文件传输走 `bundle`/目录归档通道（单 tar、单 checksum、单 ArtifactVersion）。
  像 Trinity 这样的工具，推荐用法是 spec 只指向最终产物（`Trinity.fasta`），十几
  万中间碎文件不枚举、不取回、不记录，由清理一次 `rm -rf` 回收；确需保留时整目
  录打包为一个归档产物。skill 文档（`remote-compute-ssh`）写明这一取舍。
- **调度器（SLURM）后端**：不在本计划内；清理/取回抽象以 workdir 为单位，未来调度
  器 run 可复用。
- **不做**：`transfers` 独立表（沿用 `kind='file_transfer'` 的 run）、`data_assets`
  首类表（research_nodes 现状够用）、全项目 runs timeline 页面（另行立项）。
