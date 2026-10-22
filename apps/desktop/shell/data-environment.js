const fs = require("node:fs");
const path = require("node:path");

function validateEnvironment(value) {
  if (value !== "dev" && value !== "prod") throw new Error("数据环境配置无效，只允许 dev 或 prod。");
  return value;
}

// 从调试包转为免安装正式包时，显式保留原数据环境；配置在程序目录外，后续更新继续沿用。
// 打包版本首次启动绑定本次构建的数据环境，跨通道更新继续沿用；源码运行保持原有行为。
function loadDataEnvironment({ userData, args = process.argv, defaultEnvironment = null }) {
  const selections = args.filter(arg => arg.startsWith("--adopt-data-environment="));
  if (selections.length > 1) throw new Error("不能重复指定数据环境。");
  const requested = selections.length ? validateEnvironment(selections[0].slice("--adopt-data-environment=".length)) : null;
  if (defaultEnvironment !== null) validateEnvironment(defaultEnvironment);
  const file = path.join(userData, "data-environment.json");
  let saved;
  try {
    if (fs.statSync(file).size > 1024) throw new Error("数据环境配置过大。");
    const config = JSON.parse(fs.readFileSync(file, "utf8"));
    if (!config || config.schemaVersion !== 1) throw new Error("数据环境配置版本无效。");
    saved = validateEnvironment(config.environment);
  } catch (error) {
    if (error.code !== "ENOENT") throw new Error(`无法读取原有数据环境：${error.message}`);
  }
  if (saved && requested && saved !== requested) throw new Error("已绑定的数据环境与本次指定值不一致，未切换数据目录。");
  const initial = requested ?? defaultEnvironment;
  if (!saved && initial) {
    fs.mkdirSync(userData, { recursive: true });
    fs.writeFileSync(file, JSON.stringify({ schemaVersion: 1, environment: initial }, null, 2) + "\n", { flag: "wx", mode: 0o600 });
    saved = initial;
  }
  return saved ? { CILAI_DATA_ENVIRONMENT: saved } : {};
}

module.exports = { loadDataEnvironment };
