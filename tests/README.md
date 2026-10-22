# 测试目录约束

- `contract`：协议 envelope、方法注册、Schema 和类型生成一致性。
- `integration`：App Server、SQLite、IPC 和工作区协作。
- `recovery`：异常退出、重启、事件重放和补偿扫描。
- `performance`：10,000 文件样本和长任务响应性。

每个测试运行必须创建独立临时数据目录，并显式传入环境、数据库、IPC、日志、缓存和 fixture 路径。测试不得读取或复用 production 数据目录，不得依赖开发者已有工作区，也不得在源码目录生成运行数据库。

