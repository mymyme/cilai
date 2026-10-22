# M0 测试 Fixture

Fixture 不直接提交大量生成文件，只提交确定性生成规则。生成器强制要求输出到源码树之外尚不存在的目录，防止测试数据、权限文件和性能样本污染工程或 iCloud 工作区。

```bash
FIXTURE_PARENT="$(mktemp -d)"
npm run fixtures:generate -- --output "$FIXTURE_PARENT/contexthub-fixtures"
```

生成四类样本：

- `small`：100 个确定性 Markdown 文件，用于功能和状态测试。
- `medium`：10,000 个文本文件，用于扫描、监听和性能基准。
- `abnormal`：未知编码、大文件、无读取权限、越界符号链接和长路径。
- `conflict`：同一基线产生本地与平台分叉的三方内容及 SHA-256。

生成目录只用于单次测试运行，不得包含真实客户文件、凭证或个人信息。测试必须通过显式路径接收该目录，不得扫描开发者主目录或生产 Application Support。

