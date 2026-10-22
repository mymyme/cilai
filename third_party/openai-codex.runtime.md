# 官方对话运行时

工作台通过 `openai-codex.release.json` 固定官方 Codex App Server 0.163.0 发行件，分别校验归档 SHA-256、包版本、平台和可执行文件 SHA-256。打包保持官方 `bin`、`codex-path`、`codex-resources` 与包清单的完整布局。

`openai-codex/` 继续保留仓库已有的第三方源码快照。本次启动修复不修改、也不编译该目录；源码快照与官方二进制发行件的来源分别记录。发行包准备流程不要求把已有源码替换为另一版本。

已有升级资料位于工作台内核目录的 `official-runtime` 子目录。工作台自身的适配层使用官方公开协议，并保留工作台拥有的迁入历史显示结构。目录更名的前向迁移由 Local App Server 在开放服务前执行：验证原归档与清单，额外备份子目录，只调整原生状态索引的 rollout 定位字段和工作台升级回执中的目录摘要；逐字保留会话文件与分页正文数据库，并验证其余原生状态未改变。

## 同版本许可通知

当前发行件清单另固定 `openai-codex-notices/rust-v0.163.0/provenance.json` 及其 SHA-256。该清单绑定官方标签对象、commit `83086188ae0614bd882818ff1aa23f3194962012`、根 tree 和 LICENSE/NOTICE 的 Git blob 与 SHA-256；文本来自该 commit 的官方文件，未借用旧快照来源。

准备阶段校验上游归档和程序后，再从本地固定副本复制 `LICENSE`、`NOTICE`、`cilai-codex-notices.json` 到新 `codex-runtime`。暂存和最终包均校验三文件，普通构建不联网取得通知，不重写上游程序或包描述。这是 Codex 顶层通知补齐，最终产品与嵌套依赖的完整公开权利核对仍单独进行。
