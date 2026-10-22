# 工作台技能创建器来源与维护

定制入口：`skill-creator/SKILL.md`。由 Local App Server 编译内嵌并写入当前 Profile 的受管系统技能目录。系统技能标识保持 `system.skill-creator`，修订为 2。

个人应用创建入口：`capability-creator/SKILL.md`。由 Local App Server 编译内嵌并写入当前本地资料的受管系统技能目录。系统技能标识为 `system.capability-creator`，修订为 5；交付个人 Skill、本地 MCP 或个人应用，不连接企业发布服务。

基于 OpenAI Codex 通用 skill-creator 的技能设计方法整理：保留用途与边界、渐进披露、脚本与资源选择、真实任务验证；交付与发现策略由工作台维护。

- 上游：`https://github.com/openai/codex`
- 本仓库来源：`third_party/openai-codex/codex-rs/skills/src/assets/samples/skill-creator/SKILL.md`
- 核对日期：2026-09-24
- 来源 SHA-256：`6656e54755638e8efcf275a472b9672eaa8a9a1b9e59dc210e275b03b59e1e66`
- 版权与许可证：OpenAI Codex，Copyright 2025 OpenAI，Apache-2.0；见仓库内 `third_party/openai-codex/LICENSE` 与 `NOTICE`。

Harness 升级时不覆盖此文件。更新时对比以上来源摘要，按需人工吸收上游技能方法，保留工作台交付契约。不得把工作台定制内容写入 Harness 自动生成的 `.system` 目录，也不得修改第三方源码。

升级验证：通过原生 `skills/config/write` 停用专用 Harness 目录中的原版创建器，确认成功后注册工作台运行快照；确认普通对话可发现并读取定制版、原版不参与选择、显式选择仍可用、输出可自动安装。配置接口失败必须使初始化明确失败，不能静默启用原版。

可复用的原生协议验证：运行 `node scripts/verify-skill-creator-harness.mjs <codex-app-server路径>`。独立临时环境验证首次发现、精确停用、额外目录加载、其他配置保留，以及系统技能缓存重建后的启停状态。
