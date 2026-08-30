# 隔离探索分支与主线晋升实施计划

> 设计依据：[隔离探索分支与主线晋升设计](../specs/2026-08-09-exploration-branches-design.md)
>
> 对应 Issue：[#726](https://github.com/xuzhougeng/wisp-science/issues/726)

## 交付策略

按小 PR 交付，不在一个变更中同时重写会话、Artifact、Run、文件系统和 UI。前五个 PR 构成 current-head MVP；第六个 PR 才扩展到任意历史轮次。

每个行为变更都必须带测试。任何 PR 若涉及 UI/Tauri，除窄测试外还需运行 WASM check 和 Playwright；最终 MVP 运行仓库要求的全套验证。

## PR 1：检查点、探索 scope 和 Store 迁移

**用户问题：** 先建立不会与普通会话分支混淆的持久身份、状态机和可校验 guard。

### 修改

- 修改 `crates/wisp-store/migrations/0000_init.sql`。
- 修改 `crates/wisp-store/src/lib.rs`，增加 idempotent migration/repair。
- 新建 `crates/wisp-store/src/explorations.rs`。
- 修改 `crates/wisp-store/src/models.rs` 导出 DTO/enum。
- 修改 `frames`、`artifacts`、`runs`、`research_nodes`、`research_edges`、
  `external_resources`，加入 nullable `exploration_id`；旧数据保持 NULL。
- 新增：
  - `exploration_checkpoints`
  - `exploration_families`
  - `explorations`
  - `workspace_snapshots`
  - `exploration_baseline_entities`
  - `exploration_baseline_artifact_heads`
  - `artifact_heads`
  - `exploration_effects`
  - `exploration_promotions`
  - `project_state_counters`
  - `context_archives`
- 为现有 Artifact 回填 mainline head；保留 `latest_version_id` 兼容字段。
- 幂等移除旧 `ux_artifacts_project_logical_key` 唯一索引，改为普通索引；scope 内的
  logical-key 唯一性由 `artifact_heads` 主键保证。

### 接口

```rust
Store::create_exploration_checkpoint(...)
Store::create_exploration(...)
Store::get_exploration(id)
Store::exploration_for_frame(frame_id)
Store::list_explorations(source_frame_id)
Store::transition_exploration(id, expected_status, next_status)
Store::project_state_generation(project_id)
Store::bump_state_generation(tx, state_scope)
Store::backfill_mainline_artifact_heads()
```

`bump_state_generation` 必须按 scope 分派：mainline 写入增加 project counter，
exploration 写入只增加该 exploration 的 counter。状态转换和 family mainline CAS 在 Store 层
校验，不能由 Tauri 任意更新字符串。

### 测试

- fresh database 包含新表、索引和约束。
- legacy database 重开后幂等补列/补表并正确回填 mainline Artifact head。
- 同一 frame 只能绑定一个 exploration；多个 exploration 可共享 checkpoint。
- family mainline 只能用预期 frame/generation 做 compare-and-swap。
- exploration 写入不改变 mainline generation；mainline 写入会使旧 checkpoint guard 失效。
- 非法状态转换被拒绝。
- existing `branch_session` 行为不变，旧 frame 全部是 mainline。
- migration 重跑不丢现有 Artifact、Version、Run 或 message。
- 两个 exploration 可以保存同一 logical key，且 mainline head 保持不变。

### 验证

```bash
cargo test -p wisp-store exploration
cargo test -p wisp-store artifact_head
cargo fmt --all -- --check
```

## PR 2：可移植工作区快照和持久隔离目录

**用户问题：** 两个探索必须真正写入不同目录，且不要求项目已经使用 Git。

### 修改

- 新建 `src-tauri/src/exploration_workspace.rs`。
- 从 `src-tauri/src/project_transfer.rs::collect_workspace` 提取纯扫描逻辑到合适的共享模块；只为本需求抽取，不重构无关 transfer 代码。
- 复用 `snapshot_store.rs` 的流式 SHA-256、内容寻址和 symlink 防护约定。
- 明确三个平台的复制实现：
  - macOS：优先 clonefile/reflink，失败后普通复制；
  - Linux/WSL：优先 reflink，失败后普通复制；
  - Windows：普通复制 + 同目录临时文件替换，处理 case-insensitive collision。
- 工作区放入 app data 的稳定目录；不使用 temp dir。
- 生成 `WorkspaceSnapshot` manifest、弱/强引用警告和 `.wisp/exploration-references.json`。
- exploration reference manifest 属于内部文件，diff/promotion 必须排除。
- 暂不使用 Git worktree；后续 backend 可以实现同一个 trait。

### 接口

```rust
#[async_trait]
trait ExplorationWorkspaceBackend {
    async fn checkpoint(&self, project_root: &Path) -> Result<WorkspaceSnapshot>;
    async fn materialize(
        &self,
        snapshot: &WorkspaceSnapshot,
        exploration_id: &str,
    ) -> Result<MaterializedWorkspace>;
    async fn diff(&self, base: &WorkspaceSnapshot, root: &Path)
        -> Result<Vec<FileDelta>>;
    async fn dispose(&self, workspace: &MaterializedWorkspace) -> Result<()>;
}
```

### 测试

- 从同一 snapshot 物化两个目录，修改同名文件互不影响。
- 新文件和删除只出现在对应探索。
- 未跟踪普通文件进入快照。
- writable 文件没有 hard-link inode/文件标识共享。
- symlink、socket 和超阈值文件被标为引用/unsupported，不被跟随。
- 路径包含空格、中文、Windows 保留名、大小写冲突时行为确定。
- 超过 100k entries、目录遍历逃逸、snapshot blob 损坏时安全失败。
- `dispose` 只接受 app-data exploration root 的直接子目录，拒绝项目根和任意外部路径。

### 验证

```bash
cargo test -p wisp-tauri exploration_workspace
cargo fmt --all -- --check
```

## PR 3：创建/打开探索和 branch-aware 运行环境

**用户问题：** 会话、文件、Memory、Artifact、Run 和 Decision 必须使用同一个探索 scope。

### 修改

- 新建 `src-tauri/src/exploration_commands.rs`。
- 修改 `src-tauri/src/session_commands.rs`：保留 `branch_session`；新增探索 frame 克隆路径。
- 修改 `src-tauri/src/lib.rs`：在确定 frame 后解析 `WorkingProject`，Agent cache 以 frame/root/scope 校验。
- 修改文件浏览、Artifact、Runtime、Run、Research Graph、Reader/Composer 引用命令，让它们接收或解析 `StateView`。
- 修改 `crates/wisp-store/src/artifacts.rs`：new writes 更新 scope-aware `artifact_heads`；读取 exact version，不使用项目级 latest 猜测。
- 修改 `crates/wisp-store/src/runs.rs`、Research Graph store/module 和 ExternalResource 写入，加入 `exploration_id` 过滤。
- 修改 `crates/wisp-store/src/project_transfer.rs` 和 Tauri project sync/export 查询：普通项目快照只包含主线记录，并明确报告被排除的探索数量。
- Exploration frame 使用独立 `MemoryManager` 和 branch root skill index。
- system message 用探索 root 重新生成，重新应用 Specialist、Delegation/Plan 配置；不复制 source runtime。
- 新 compaction 使用 `context_archives` 逻辑引用；为旧绝对路径增加受控兼容解析。
- MVP 明确拒绝 ACP exploration、历史非 head checkpoint、活跃 turn/Run 和未 scope-aware 的项目级写命令。
- 探索轮冻结 source main 的消息、分支创建、移动和删除；其他普通 conversation 只允许讨论和只读检查，ACP 绑定会话在冻结期间直接拒绝 turn。
- 普通 main 存在对话分支时禁止删除，必须先删除分支。
- 活跃探索内的 project sync/export/import/delete/settings mutation 返回稳定错误码。

### 命令

```text
create_exploration_checkpoint
create_exploration
list_explorations
open_exploration
discard_exploration
abandon_exploration_round
```

### 测试

- 从一个 checkpoint 创建两个 exploration frame，消息只克隆到完成边界。
- resource links 绑定同一个 immutable baseline ArtifactVersion。
- 两个 Agent 的 root、Memory dir、skill root 和 file panel root 不同。
- 探索 A 写文件、Artifact、Run、Decision 后：主线和探索 B 查询均不可见。
- 相同 `logical_key` 在探索内生成私有 head，不改变 mainline head。
- 切换 mainline/exploration 后运行中另一会话的事件仍按 frame 路由。
- 重启后 exploration frame 从 SQLite 和持久 workspace 恢复。
- ACP/source busy/history-without-checkpoint 返回稳定错误 code。
- 外部副作用记录到 `exploration_effects`，且审批文案说明不可回滚。

### 验证

```bash
cargo test -p wisp-store exploration_scope
cargo test -p wisp-tauri exploration_commands
cd ui && cargo check --target wasm32-unknown-unknown
cd ../ui-tests && npx playwright test --grep "exploration"
```

## PR 4：差异、guard、晋升日志和丢弃

**用户问题：** 只有主线仍等于检查点时才能采用；失败不能留下半套主线。

### 修改

- 新建 `src-tauri/src/exploration_promotion.rs`。
- 扩展 `project_activity`，提供项目级独占写锁和“项目正在晋升”状态。
- 增加 base/current/exploration 三方 manifest 比较，但只允许 `current == base` 的 fast-forward。
- 实现结构化 `ExplorationDiff` 和 `PromotionEligibility`。
- 实现 promotion journal、staging、逐文件原子替换、trash rollback 和启动恢复。
- 在一个 Store transaction 中采用 Artifact heads、Run/Decision/Resource、探索 frame 和 generation。
- 晋升后驱逐 source/selected exploration 的 Agent、runtime、Memory/skill cache，用主线 root 重建。
- 实现 `discard_exploration`：先检查无活动 turn/Run，再硬删除探索私有目录、对话和记录，不保留终态卡片。
- 晋升在同一个元数据事务中永久丢弃并清理同轮所有未采用候选。
- 实现 `abandon_exploration_round`：检查同轮无活动 turn/队列/终端/Run，原子清理全部候选并推进 family generation，但保留原 main。
- 失败或单独丢弃一个候选都不得解除仍有其他候选的 main 冻结。

### 命令

```text
preview_exploration_diff
preview_exploration_promotion
promote_exploration
discard_exploration
abandon_exploration_round
```

### 测试

- mainline 被 Wisp 冻结且外部状态未变化时，探索新增/修改/删除文件和 Artifact head 一起采用。
- 原 main frame ID 和普通分支关系保持不变，所选探索在检查点后的上下文被迁回原 main，重建 Agent 后下一轮直接看到探索上下文。
- 其他探索的消息、Artifact、Run、Decision 不进入主线查询或 prompt。
- source frame 新消息由统一 guard 拒绝；所选探索涉及的同路径主线文件被外部进程修改时触发 `MainlineAdvanced`，非重叠文件保持原样且不进入冲突预览。
- preview 后、commit 前主线变化仍被锁内二次校验拒绝。
- branch/reference 大文件 fingerprint 改变时拒绝。
- 活跃 turn、queued turn、Run、sync 和第二个 promotion 都阻止晋升。
- 对每个 journal 阶段注入失败，验证回滚或启动恢复到确定状态。
- Windows 目标文件占用、rename 失败、case collision 不破坏主线。
- 丢弃探索 A 后主线和探索 B 文件/记录校验和不变。
- 单独丢弃一个候选后，只要同轮仍有候选，main 就继续冻结；显式放弃整轮后原 main 恢复可写且所有探索产物被清理。
- source main 和带普通对话分支的 main 均不能被删除；结算探索或先删除分支后才允许删除。
- 路径校验阻止删除 mainline root、HOME、app-data root 或非直接子目录。

### 验证

```bash
cargo test -p wisp-tauri exploration_promotion
cargo test -p wisp-store exploration_promotion
cargo fmt --all -- --check
```

## PR 5：用户界面、Escape stack、文档和 MVP 端到端

**用户问题：** 用户能清楚区分普通对话分支与安全探索，并理解能否晋升及原因。

### 修改

- 在完成轮次菜单加入“开始探索”，保留现有“分支”。
- 侧栏在 source session 下展示 exploration group、状态和隔离等级。
- Exploration header/banner 增加“查看差异”“设为主线”“丢弃”。
- 主线 banner 显示探索数量、冻结原因，以及“选择候选”或“放弃探索”两条结算路径。
- source main 右键菜单提供“放弃探索”，探索未结算时和普通 main 有分支时均隐藏删除。
- 新增差异/晋升/丢弃 dialog，展示 Files、Artifacts、Runs、Decisions、External effects。
- 修改 `ui/src/dto.rs`、`ui/src/app_support/settings.rs`、`ui/src/main.rs`、`ui/src/session_modals.rs`、`ui/src/sidebar.rs`、i18n 和对应 CSS；按实际责任边界放置，不为缩短大文件做无关拆分。
- 修改 `ui-tests/tests/mock-tauri.ts` 构造两个探索及差异状态。
- 更新用户文档；不更新 release notes，除非进入发布准备。

### Playwright 场景

1. 从当前主线节点创建探索 A、B。
2. 打开 A/B 时 Files 和 Artifact 列表互不相同；切回主线保持原样。
3. 冻结的 source main 不能发送或删除；A 可设为主线，采用后 transcript 和 Artifact 同步切换，并清理 B。
4. B 内容不出现在主线。
5. 单独丢弃一个候选后，只要仍有其他候选，就不解除 source main 冻结。
6. 从 source main 右键选择“放弃探索”后清理 A/B，并恢复原 main 可写。
7. 每个 dialog 打开后立即按一次 Escape，只关闭最上层，父 overlay 保持。
8. 紧凑窗口中的差异 dialog/Inspector drawer 可用。
9. 中英文文案都不把晋升称为通用 merge。

### 最终验证

```bash
cargo fmt --all -- --check
cargo test --workspace
cd ui && cargo check --target wasm32-unknown-unknown
cd ../ui-tests && npm ci && npx playwright test
```

### 手工 smoke

- macOS 非 Git 项目：创建两个探索，修改同文件，采用一个。
- Windows 项目：重复上述流程，包含空格/中文路径和被占用文件的失败提示。
- Git dirty + untracked 项目：确认无需 stash/commit 即可创建探索，主线 Git index 未改变。
- 创建远程 Run：确认外部副作用警告，丢弃后远程作业不会被声称回滚。
- 重启应用：活跃探索和 promotion recovery 状态保持正确。

## PR 6：任意历史轮次检查点（MVP 后）

**用户问题：** Issue 期望从任意已完成节点探索，但旧系统没有对应的项目状态历史。

### 修改

- 新增不可变 `project_state_revisions`，在每个成功持久化的完成 turn 后记录：
  - parent revision；
  - frame/message/UI event 边界；
  - workspace manifest delta；
  - Artifact head set；
  - Run/Decision membership；
  - external effect summary。
- 内容 blob 去重，revision 只保存增量和周期性 full manifest。
- “开始探索”从选中 turn 查找 revision；没有 revision 的升级前历史明确不可用。
- 后台外部文件变化在 turn boundary 扫描并记录，不假设全部写入都来自工具 provenance。

### 测试

- 三个完成轮次分别修改同文件，从第二轮创建探索得到第二轮内容而不是当前内容。
- compaction 前后 message boundary 仍映射到正确 revision。
- 外部编辑器在两轮之间的变化进入下一 revision。
- 升级前 turn 无 revision 时禁用并提供从当前头开始的替代动作。
- 长会话 revision 数量增加时 blob 去重和加载边界可控。

## 之后的明确 follow-up

- 选择性采用单个文件或 Artifact，但不合并对话。
- Git worktree backend 优化，复用现有 `GitCommandRunner` 和 patch preflight。
- 大型 DataAsset 的只读 mount/resolver。
- ACP 协议具备 server-side fork 后再开放 ACP exploration。
- 探索专用导出/导入和跨设备同步。
- 多个 project window 的探索状态实时刷新和恢复冲突处理。

## PR 描述模板

每个 PR 应包含：

- 解决的用户问题和本 PR 的明确非目标；
- 新增 schema/抽象以及迁移兼容方式；
- 修改文件列表；
- 自动测试和手工 smoke；
- Windows/macOS 差异；
- 大文件、远程副作用和崩溃恢复限制；
- 下一 PR 才处理的内容。
