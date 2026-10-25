# 词来工作台工程约束

- 始终用中文回复；修改前先分析根因与影响范围。
- 本工程是 ContextHub 桌面端的独立复制；不得写入或清理来源工程、原应用数据及原构建目录。
- `domain` 不依赖 Electron 宿主、SQLite、文件监听库或平台专属 API。
- UI、CLI 不直接访问 SQLite、工作区文件或企业平台 API；Local App Server 是业务状态和 SQLite 的唯一写入者。
- 公开架构与边界位于 `docs/ARCHITECTURE.md`；构建和兼容状态分别见 `docs/BUILDING.md` 与 `docs/COMPATIBILITY.md`。完整个人工程内的需求及验收文档仍位于工程根 `02需求`。
- 架构边界变化同步更新架构文档；修复验证通过后，以业务语言补充对应验收标准。
- 不加入静默降级或兜底路径；失败需要明确状态。
- 不自动建立 Git 仓库或分支，不将来源项目的仓库管理规则沿用到此目录。当前公开仓库为 `https://github.com/mymyme/cilai`；用户已明确授权本次源码提交和推送，后续 Git 操作按具体任务授权办理。
- 内部 contexthub crate、协议、清单文件名及历史数据库迁移保留；新增版本按前向迁移演进。用户工作区元数据目录统一使用 `.cilai`，不得新建 `.contexthub`；旧目录仅在当前工作区清单归属与校验通过后迁移，新旧目录冲突必须明确报告并保留数据。
- 个人版唯一构建根为用户主目录下 `.cilai-personal-builds/desktop`，由 `scripts/personal-build.mjs` 和根内显式登记文件校验源码归属；构建根必须位于非云同步的本地目录，不能经过符号链接。源码仍可位于 iCloud，缓存及产物不再写回项目内 `.personal-build`；安装名“词来个人工作台”、appId `cn.cilai.personal.workbench`、可执行名 `cilai-personal-workbench`。禁止使用任何企业版构建根或依赖目录联接。符号链接越界或构建根路径异常时必须停止。
- 既有原生验证过渡目录及登记仅作为历史证据保留，不新增旁路构建根。新的打包、原生测试和运行时夹具统一使用已登记的个人本地根；旧云目录中的历史现场不自动清理，退役标记禁止继续构建。
- 构建暂存按当前个人源码扫描，不要求 Git 初始化、分支或提交；排除 .git、.venv、node_modules、运行数据及个人配置。不能复制企业成品或隐式读取仓库外共享源码。
- `npm run package` 生成本地测试包；macOS 使用临时签名、不读取维护者证书，在线更新禁用。正式签名、发布、portable 与在线更新测试入口在个人版独立源和发布授权完成前明确拒绝执行。
- Windows 最低兼容基线为 Windows 10 22H2 x64，并支持其后的 Windows 客户端 x64 版本，包括 Windows 11；家庭版、专业版和企业版均在兼容范围内，兼容范围不按微软生命周期、ESU 或 LTSC 类型判断。Windows 与 macOS 同等重要，两端共用领域、应用、持久化和协议代码，各自独立验收，验收结论互不移植。当前已有部分 Windows 源码与静态契约证据；Windows 原生构建、安装、启动和功能结论仍须独立实机验收，不以 macOS 回归或交叉检查产物交付。
- macOS 与 Windows 的改动照常推进，不设 macOS 既有行为冻结期；改变既有行为的改动按正常流程分析影响、同步文档与验收标准，并在两端分别回归。
- 平台条件编译和系统调用只允许出现在 `crates/local-transport` 与 `crates/platform-adapter`，且按平台正向列举，不以否定分支承载任何平台的正式实现。其余各层一律经平台适配接口访问系统能力；不依赖系统权限的能力优先在前端实现。评审须拦截平台代码向外扩散。
- workspace 默认拒绝 `unsafe`，业务 crate 继续禁止 `unsafe`。只有 `crates/local-transport` 与 `crates/platform-adapter` 的 Windows 私有模块可为 DACL、SID、Job Object 等必要 Win32 调用使用最小范围的 `unsafe`；对外只暴露安全接口，不暴露原始句柄、指针或平台结构，每个调用点须说明安全前提并由 Windows 契约测试覆盖。
- 平台能力以能力矩阵为契约，见 [公开架构与边界](docs/ARCHITECTURE.md)。新增能力同时登记各平台状态，平台缺失能力返回明确状态，不得静默降级。
- Web应用必须内嵌在工作台主窗口的应用标签页中，使用 Electron 受控内容进程，不创建独立业务应用窗口。主界面只能通过预加载白名单调用宿主能力；应用内容进程不获得 Node 能力或通用宿主 IPC，只能使用 App Bridge 私有通道。
- App Bridge的`session`、`context`、`uiAction`、`model`和`audit`用于应用页面内临时AI，`conversation`只处理用户主动转交；个人应用工具由获授权的工作台主对话和官方运行时直接调用，不经过页面Bridge。两条链路在Windows与macOS共用Schema、权限、错误和验收口径，不得做平台裁剪。
- 新增依赖以跨平台覆盖度为选型硬条件。
- 个人版不得默认配置或启动企业、云空间、协作和采集服务；企业版凭据、配置和历史不属于本工程实施范围。
- third_party 是第三方源码，保留其许可证和来源记录；不要做品牌替换。
- 自有随包程序统一使用 cilai 名称；正式及应用内核数据目录分别为 ai-runtime/cilai-harness 与 ai-runtime/cilai-application-harness。旧目录迁移由 Local App Server 在开放服务之前完成，保留来源备份并校验历史路径，不增加旧名查找入口。上游包名、临时执行别名、系统技能标记、插件及 Windows 沙箱约定保留。
- 桌面握手内部名称固定为 `contexthub-desktop`；应用品牌和主进程可独立命名。构建产物不得从其他目录直接复制复用。

- Windows 工具链使用调用者已安装的 Node、Rust 和微软 C++ Build Tools，`CILAI_PERSONAL_MSVC_ROOT` 可显式指定工具链读取路径；下载缓存、npm 依赖、临时文件和产物均位于个人版构建根，不修改机器级配置、不建立指向企业目录的联接。
- Windows 与 macOS 独立验证；来源工程的实机报告不能作为个人版通过证据。当前构建输出按 testing/stable 固定目录隔离，正在使用的个人版包必须正常退出后重建，不能批量清理同名进程。

- 构建、专项验证和完整备份遵循 [空间管理流程](docs/BUILD_STORAGE.md)。专项验证使用登记入口；不得按任务编号新增整套编译缓存或遗留重复 sidecar；业务备份必须有真正的源写保护，按数据集保留最近两份已完整验证备份。禁止自动扫描删除未知目录或通过终止现用应用绕过打包检查。
