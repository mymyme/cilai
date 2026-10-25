# 本地 MCP 隔离边界与剩余实施

审计日期：2026-10-25。本说明区分任意外部 MCP 命令与工作台固定 Node 个人包，两者不共享能力结论。

## 当前能力

| 能力 | macOS | Windows |
| --- | --- | --- |
| 任意外部本地命令，禁止子进程 | 已有 Seatbelt 文件、网络与执行限制；核验入口见下文 | 明确拒绝 `mcp_local_isolation_not_supported` |
| 任意外部本地命令，允许子进程 | 明确拒绝 `mcp_child_processes_not_supported` | 明确拒绝 `mcp_local_isolation_not_supported` |
| 工作台固定 Node 个人包 | 独立的受管组件沙箱 | 独立的工作台 LPAC 启动器；不构成任意命令支持 |
| 本地命令代理配置 | 非 direct 配置明确拒绝 | 非 direct 配置明确拒绝 |

拒绝行为保留在 `crates/platform-adapter/src/mcp_sandbox.rs`。本轮没有移除拒绝、增加裸进程启动路径或放宽企业资料访问。固定 Node 路径位于 `managed_component.rs` 和 `managed_component/windows.rs`，其可信入口、只读包及私有资料范围均固定，不能作为外部程序的通用授权器。

## 已审计的调用链

`mcp_connections/local.rs` 取得当前执行租约，准备私有运行目录，调用 `Plan::capture_managed` 校验命令、脚本、目录和权限。支持的平台生成隔离命令后，通过工作台写者启动门启动。核对真实进程身份并登记租约之后才发送许可，随后经专用标准输入输出交换 MCP 消息。失败及未知副作用结果不能视为成功。

macOS 的 `Plan` 校验原始与规范路径、入口及脚本摘要、用户授予的读写范围、受管命名空间和原始官方运行时目录。`network=false` 时不授予网络能力；`network=true` 是整体网络许可，不是按域名的代理隔离。本地命令的系统代理及显式代理配置在持久化入口被拒绝，不能将远端 HTTP MCP 的代理实现当成本地命令代理支持。

Windows 通用进程监督使用挂起启动、Job 分配后恢复及关闭 Job 时终止成员。它解决进程生存期，不提供文件、凭据或网络隔离。固定 Node LPAC 启动器另外设置包 SID、LPAC 属性、标准句柄白名单、私有桌面和目录权限，并限制单进程及内存。它会为受管包及其私有目录添加隔离身份权限，没有针对任意用户目录的授权撤销事务。因此不能把调用者选择的外部工作区直接交给该启动器。

## Windows 任意命令的实施前提

1. 独立通用启动计划：明确可执行文件、脚本和 DLL 依赖、读写路径、私有 HOME、网络与子进程权限；拒绝受管业务数据、其他连接资料、重解析点、未知路径及来源变化。
2. 每个租约的隔离身份：真实以 AppContainer/LPAC token 创建进程，核对 token；只授予本次确定的能力，不继承宿主凭据或通用句柄。文件路径 ACL 与 token 是两套检查，不能只设置其中一套。
3. 可核对的外部目录授权事务：记录原有安全描述符与所添加权限，在不改变属主、不抹掉拒绝规则的条件下授予最小范围。退出或恢复时仅撤销自己的授权；并发外部变更、崩溃和撤销失败必须保留明确状态。不能清理其他任务或未知目录。
4. 网络边界：不联网时由系统能力实际拒绝；联网时明确互联网、局域网、回环及代理范围。若使用代理，必须证明不能绕过代理直连；不能以环境变量代替隔离。
5. 生存期与启动门：创建时即受 Job 约束，不留先执行后登记窗口，不允许子进程 breakaway。子进程权限关闭时同时使用系统子进程策略和进程数约束；允许时所有后代仍处于相同文件、网络与租约范围。
6. 标准输入输出、中文路径、退出码、取消、超时、宿主异常退出及临时授权恢复均完成后，才能接入 `mcp_connections/local.rs`。任一条件无法证明时保留明确拒绝。

## macOS 子进程的实施前提

Apple 明确说明 fork 子进程继承静态沙箱；这支持文件和网络限制的继承，不能单独证明子树能够完整停止。当前 Unix 通用监督没有持有整棵进程树的系统对象，`Session` 退出主要终止根进程并等待读取线程。

单纯加 `process-fork` 或普通进程组不充分：子进程可以尝试 `setsid`、双重 fork 或脱离进程组，继续持有资料权限与协议管道。Apple XNU 的 `setsid`/`setpgid` 实现不能作为 Seatbelt 禁止脱离的保证；本机策略编译也不接受 `process-setpgid`。不得以“已发送终止信号”或“父进程已退出”代替全部写者退出证据。

启用前须实现并验证以下约束：可执行目标不超出明确授权；子进程继承相同文件和网络策略；不能改变或脱离隔离；主进程退出、取消、超时和宿主异常时后代均被停止；持有标准管道的孙进程不能阻止结束或在结束后继续写入。没有完整生存期证明前保留当前拒绝。

## 独立实机验收清单

- 允许目录可读、只读目录不可写、允许写目录可写；业务数据库、原始凭据、其他 MCP 私有目录和未授权用户文件均无法访问。
- 路径链接、硬链接、重解析点、入口替换与脚本变化不能突破范围；拒绝后没有执行副作用。
- 禁止网络时，互联网、局域网、回环及代理连接均被实际拒绝。允许网络时，实际行为与已确认权限一致。
- 关闭子进程权限时无法创建后代；允许时验证至少两代后代及脱离、继承句柄、执行其他程序的尝试。
- 结束、取消、超时、宿主被终止及机器会话结束后，全部相关进程退出；回执、管道及授权撤销均有可核对结果，未知结果阻止后续维护。
- 外部目录原有属主、允许与拒绝规则保持可用；撤销、崩溃恢复及并发 ACL 变更不会放大权限或误删用户资料。
- Windows 10 22H2 x64 与后续客户端 x64 分别核验创建、实际限制和回收；静态检查、交叉编译或 macOS 结果不能替代 Windows 实机通过。

已有测试入口：`platform-adapter/src/mcp_sandbox.rs` 的内核权限探针、`managed_component/windows.rs` 的固定 Node LPAC 探针，以及 `workbench-runtime/src/mcp_connections/tests.rs` 的租约、标准管道与不支持权限拒绝测试。最初外层受限环境中的独立 `sandbox_apply` 探针返回 `Operation not permitted`。随后在获准的原生测试环境中，实际生产沙箱模块的文件/网络/子进程拒绝、许可网络、原目录及链接别名保护测试通过；测试仅使用合成文件和本机端口。此结果不覆盖仍被拒绝的子进程权限、Windows 任意命令或完整外部服务兼容。

## 官方依据

- [Microsoft：AppContainer 隔离](https://learn.microsoft.com/en-us/windows/win32/secauthz/appcontainer-isolation)
- [Microsoft：启动 AppContainer/LPAC](https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer)
- [Microsoft：进程创建属性与标准句柄白名单](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)
- [Microsoft：Job Objects、后代归属和 breakaway](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
- [Apple DTS：fork/exec 与静态沙箱继承](https://developer.apple.com/forums/thread/747499)
- [Apple 官方 XNU：会话与进程组系统调用](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_prot.c)
