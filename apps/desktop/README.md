# 词来个人工作台桌面宿主

本目录承载个人版 Electron Shell 和 Rust Host。个人版本地资料、模型授权、浏览器状态、构建输出与更新配置均独立。完整入口与工具要求见[源码构建说明](../../README.md)，实际结果见[公开兼容记录](../../docs/COMPATIBILITY.md)。来源工程的旧登录、企业平台、共享数据目录及历史实机记录不作为本版操作说明或通过证据。

## 安全的本地构建入口

从源码根执行：

```sh
npm run check:toolchain
node scripts/personal-build.mjs
npm run package
```

构建器在个人工程根的 `.personal-build` 中按当前源码清单暂存、安装锁定依赖、编译自有服务与界面、准备运行时并生成 testing 本地包。不要直接沿用旧企业构建路径，不要在企业依赖目录建立链接，不需要先在源码目录安装桌面依赖。macOS 本地包使用临时签名；正式签名、发布、在线更新和 portable 流程尚未开放。

`npm run launch` 使用同一独立构建入口后打开个人测试包。不要以本目录的原型或裸 Electron 命令代替完整产品启动验证。重建前仅正常退出当前个人测试包；不按名称批量停止进程。

| 项目 | 个人版约定 |
| --- | --- |
| 应用标识 | `cn.cilai.personal.workbench` |
| 安装名称 | 词来个人工作台 |
| 可执行名 | `cilai-personal-workbench` |
| 源码运行根 | 个人工程根 `.personal-run/cn.cilai.personal.workbench` |
| 包运行根 | 系统应用数据根下 `cn.cilai.personal.workbench` |
| 模型秘密服务 | 工作台资料内本地加密库 model 命名空间，按资料/连接/代次隔离 |
| 网络秘密服务 | 工作台资料内本地加密库 network 命名空间，与模型主密钥分离 |
| 更新 | local 空源，不使用企业通道 |

## 代码归属

- `shell/main.js` 管理可信主窗口、生命周期和本机能力；`preload.js` 只开放固定白名单。
- `shell/service.js` 启动本版 Host；`host-service` 适配工作台 Runtime，业务资料由 Local App Server 通过 IPC 唯一写入。界面和页面内容进程不直接访问 SQLite。
- 个人应用内嵌主窗口应用标签页。页面只取得已声明、用户确认且与真实框架绑定的私有能力；不获得通用宿主 IPC、任意 Node 或其他应用工具。
- 模型连接、网络、凭据、维护、通知和页面动作在执行前复核真实资料与当前授权；取消、失效和未知回执明确呈现，不自动替换连接或重做副作用。
- 本机通知默认关闭。点击只选择仍存在的对话、规则或同一应用文档；重载、资料变化及框架失效后重新核对。
- Host 退出与全部相关写者、模型租约、浏览器/应用回执分别核对；不能用界面完成状态放行维护。

固定官方 Codex App Server 0.163.0 由源码根运行时清单约束，保留的0.153.4源码快照不是本版实际执行件。不得修改上游源码、复制原有 `~/.codex` 认证或复用企业资料。官方ChatGPT授权采用隔离原生目录的运行时托管设备码生命周期，真实账号授权仍待验证；API、Responses与本地模型按实际组合分别验收。

## 验证与交付

以下命令从源码根执行，仅表示对应检查层：

```sh
npm run typecheck
npm --prefix apps/desktop run test:shell
npm run test:platform-boundary
npm run test:build-config
npm run test:stage-binaries
```

真实桌面启动、系统权限、通知、模型调用、工具结果和正式分发分别留证。macOS 的组件或包成功不能证明 Windows 实机通过；Windows 最低目标为10 22H2 x64，并覆盖后续客户端x64。没有Windows环境或真实授权连接的条目继续标记待验/受阻，不写为成功。

- [公开架构与边界](../../docs/ARCHITECTURE.md)
- [本地构建指南](../../docs/BUILDING.md)
- [兼容状态与已知缺口](../../docs/COMPATIBILITY.md)
- [第三方来源](../../docs/THIRD_PARTY.md)

第一方源码已采用 Apache-2.0；完整第三方分发审核、双端干净构建与正式签名发行仍分别待验。企业版官网：[www.cilai.com.cn](https://www.cilai.com.cn)。
