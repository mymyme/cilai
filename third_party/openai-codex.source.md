# Retained OpenAI Codex source snapshot

- Repository: `https://github.com/openai/codex`
- Tag: `rust-v0.153.4`
- Commit: `3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`
- Imported as: source archive without nested Git metadata
- Historical target: `codex-rs/app-server` (`codex-app-server` binary)
- License: Apache-2.0; bundled from `third_party/openai-codex/LICENSE`
- Cargo lock normalization: only workspace package versions were changed from the release archive placeholder `0.0.0` to `0.153.4`; all external dependency versions and checksums remain those of the tagged archive

This retained 0.153.4 snapshot is used for historical regression and reference. The current workbench runtime is the complete official 0.163.0 release pinned by third_party/openai-codex.release.json. Packaging verifies the official archive and executable hashes and does not compile this snapshot or include its local patches. Updating either target requires its own explicit source or release provenance and acceptance evidence.

## Retained snapshot integration patches

- Consolidation explicitly grants the account memory root and its `.worker-tmp` directory as writable roots. Windows protected child DACLs may not inherit the parent grant; a read-only TEMP forces PowerShell into constrained language and breaks UTF-8 output initialization. System temporary directories and network access remain excluded.
- Workbench-managed memory homes record extraction-recovery policy and model-configuration fingerprints. A changed fingerprint rearms only exhausted failed stage-1 attempts without clearing errors, backoff, source watermarks, successful outputs or active ownership. Previously failed sources may be claimed outside the new-source age window, while source isolation, idle age, ownership, backoff and per-startup limits still apply. Original timestamps are never changed, and the same configuration cannot repeatedly rearm failures.
- Filesystem rollout lookups without a state database are expected for ephemeral memory agents and do not emit state-database discrepancy warnings. Actual persisted-index discrepancies retain their validation and diagnostics.

- Include the `contexthub` interactive source in native memory extraction candidates, matching custom-source filters to their persisted JSON representation.
- Track startup extraction and consolidation through completion; experimental `memory/flush` waits for this process's memory tasks, with native stream idle timeouts, consolidation status monitoring and orderly agent shutdown. The desktop delivers its main reply before waiting and retains the original model-session proxy until exit.
- Upgrades must retain source eligibility, model overrides, memory reset semantics and the flush lifecycle contract.

- Workbench-managed homes opt into atomic, immutable memory publication using `memory-publication`; draft files never feed the read path, and explicit notes remain writable in the working directory. Reset clears all published versions and takes the same process-wide file lock as generation.
- Skip consolidation without source material. Bound extraction inputs and consolidation token usage, retain retry/lease cleanup, and persist stage/error/publication status for the Workbench settings UI.
- Extraction results commit independently; failures no longer block consolidation of successful inputs. Report partial failures with counts and the first error while preserving readable publications. Lease and retry durations follow the native one-hour policy with a 90-second consolidation heartbeat; there is no fixed extraction/consolidation wall-clock deadline. Prompt templates and shared model selection are unchanged.

- Memory extraction and consolidation apply stage reasoning-effort preferences only when supported by the selected model catalog entry. Otherwise native request construction uses the model-declared default; models without reasoning controls receive no effort parameter. The shared model and prompt templates are unchanged.

## Local Windows patch (2026-09-26)

Sandbox setup and ACL refresh pass the originating process token's user SID to the
elevated helper. The helper accepts that exact SID instead of looking up a bare
`USERNAME`, which can resolve to the machine SID when the account and computer
share a name. Explicit provisioning for another user retains its caller-supplied
identity. Sandbox access masks, elevation requirements, and macOS behavior are
unchanged.

The patch is confined to `codex-rs/windows-sandbox-rs`: `token.rs`, `setup.rs`,
`bin/setup_main/win/sandbox_users.rs`, and the existing token/ACL test files.
Regression coverage checks the originating token identity, exact SID resolution,
and owner access after locking a sandbox directory. Retain or reassess this patch
when updating the pinned upstream snapshot.

## Local Windows sandbox credential patch (2026-10-18)

The sandbox users (`CodexSandboxOffline` / `CodexSandboxOnline`) are machine-wide
local accounts, but upstream stores their DPAPI-protected passwords under each
`CODEX_HOME/.sandbox-secrets`. With several Workbench data environments or
accounts on one Windows user, every elevated setup resets the shared passwords and
invalidates the copy held by every other `CODEX_HOME`.

`sandbox_secrets_dir` honours an absolute `CODEX_WINDOWS_SANDBOX_SECRETS_DIR`; the
Workbench sets it to `%LocalAppData%\com.cilai.workbench\windows-sandbox-secrets`
(resolved through the Known Folder API). The caller resolves the directory into
the elevation payload (`secrets_dir`), because the elevated helper does not inherit
the caller's environment; payloads without the field keep the upstream location.
Directory ACLs, DPAPI protection, setup markers (still per `CODEX_HOME`) and
macOS behaviour are unchanged. Files: `windows-sandbox-rs/src/setup.rs`,
`bin/setup_main/win.rs`, `bin/setup_main/win/sandbox_users.rs`. Retain or reassess
when updating the pinned upstream snapshot.

## Local memory isolation patch (2026-10-12)

- Consolidation disables hosted web search and clears foreground search configuration, context window and compaction limits; the selected memory model resolves its own capabilities. Existing local memory tools and prompt templates remain intact.
- A terminal foreground thread may be unsubscribed and confirmed unloaded before `memory/flush` finishes. The independent memory agent continues with the original process and proxy; foreground conversation ownership is no longer held for background completion.
- Deterministic errors requiring model configuration changes are retained with a fingerprint of memory models, injected catalog and worker tool policy. An unchanged fingerprint suppresses duplicate memory requests; changed configuration re-enters native eligibility and backoff checks. Successful completion clears the marker, and reset removes it with the publication state.
- Upgrade checks include foreground shutdown during slow consolidation, absence of hosted search tools in memory requests, immutable publication, configuration-error gating and preservation of the previous published version.

工作台会话来源 contexthub 在最后一个订阅者离开且回合空闲后立即卸载原生线程，其他来源保留上游三十分钟空闲期限；沿用原生关闭、并发订阅保护及持久化恢复流程。随包验收覆盖取消订阅后关闭、原线程恢复及后台记忆继续完成。

记忆整理累计输入与输出预算以 480,000 tokens 为基础，加上有效材料估算 tokens 的二十四倍，最高 2,400,000 tokens，包含多轮重复输入；保留原生流空闲超时、失败退避和产物校验。

## 子命令审批结果补丁（2026-10-19）

已登记的子命令审批被拒绝或取消时，在中断前记录所属父命令，并在父命令完成事件中保留拒绝状态；该事件沿用原持久化流程，历史读取与即时通知一致。状态限于当前回合，不改变权限、审批条件、标准输入审批或命令执行方式。涉及 core 的会话审批、工具完成事件和私有回合状态模块；升级上游时核对此补丁及子命令拒绝、取消和历史读取回归。

扩展测试通过独立临时配置目录运行，避免辅助进程加载用户环境文件。macOS ARM64 的安全版 V8 使用锁定版本源码构建，所需 GN、Ninja、ICU 数据和 Chromium Rust 源码按上游依赖声明取得并验证内容标识；保留全部许可证与原依赖版本，不将该测试宿主加入现有业务包。

V8 独立试验组件在 macOS 构建中声明其所需的 CoreFoundation 和 Security 框架，避免依赖其他组件间接提供链接参数。全工作区回归显式启用试验组件的 sandbox 标记，使其与代码执行运行时已经启用的同一 V8 沙箱配置一致；不关闭安全特性，不改变现有业务包的组件集合。

- 扩展测试流程同步：技能脚本验收在已经收到完成事件后直接核对结果；禁止读取验收独立确认拦截子命令的审批，再检查读取仍被阻止。只调整测试流程，不变更审批与沙箱策略。

完整工作区测试隔离用户主目录及终端颜色设置。终端单元测试使用固定版本样本，实际编译版本不变；MCP 服务握手预期核对真实构建版本，安装前缀单元测试不依赖本机安装布局。

原生状态库的 0053 前向迁移重建三个可见线程排序索引，索引条件与现有“有预览或有名称”的查询一致，不改写线程及消息或改变可见性规则。升级上游时核对此索引与列表筛选、排序及迁移保留测试。

状态库构建脚本显式追踪全部迁移目录，新增迁移时触发重新编译，避免稳定编译器的宏依赖集合仍沿用旧目录内容。消息记录单元夹具关闭假供应商的 WebSocket 能力，避免真实外部连接预热阻塞消息事件断言；产品供应商能力不变，WebSocket 集成用例继续正常运行。外部插件来源推断及导入使用测试子进程独立 Git 配置，将唯一指定的官方源解析到本轮合成仓库，实际执行克隆、登记和插件缺失核验，不修改用户 Git 或产品下载来源。

子命令取消的完成项由现有 RegularTask 中断清理统一持有并持久化，正常工具结束不得重复发布同一取消完成项；只处理当前轮次已经登记且明确取消的父命令，保留原取消令牌和中断时限。宿主使用明确的拒绝摘要，原未成功分类及业务状态机不变。测试必须收到完成事件，并在正常退出和重启后读取唯一的同项拒绝记录，不通过超时后的替代路径验收。

隔离用户目录后，扩展入口在测试开始前显式解析 DotSlash 的工具缓存、准备锁定 zsh 并核验启动和命令拦截能力；不把下载失败作为测试跳过。测试个人配置继续隔离，工具缓存按锁定清单使用。外部插件合成 Git 来源仅作用于该用例独立子进程，其他命令测试不继承重定向配置。

本轮保留快照的取消完成记录、索引迁移和扩展构建修复仅在该快照的专项测试中验证，不进入官方 0.163.0 的正式运行包。现行工作台只通过自己的协议适配与摘要展示使用官方运行时；包内系统验收必须实际运行官方发行程序，不能用快照测试代替。
