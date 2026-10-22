const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { createServiceLog } = require("./service-log");

function tempLog() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "cilai-service-log-"));
  return path.join(directory, "logs", "host-service.log");
}

test("按行带时间戳记录，跨块拼接的行完整写入", () => {
  const file = tempLog();
  const log = createServiceLog(file);
  log.write("ai_send_timing stage=run_start ");
  log.write("stage_ms=12\n\nai_state_persist_timing service_ms=3\n");
  const lines = fs.readFileSync(file, "utf8").trim().split("\n");
  assert.equal(lines.length, 2);
  assert.match(lines[0], /^\d{4}-\d\d-\d\dT[\d:.]+Z ai_send_timing stage=run_start stage_ms=12$/);
  assert.match(lines[1], /ai_state_persist_timing service_ms=3$/);
});

test("超过上限时轮换为一份旧文件", () => {
  const file = tempLog();
  const log = createServiceLog(file, 80);
  log.write(`${"甲".repeat(20)}\n`);
  log.write(`${"乙".repeat(20)}\n`);
  assert.match(fs.readFileSync(`${file}.1`, "utf8"), /甲/);
  const current = fs.readFileSync(file, "utf8");
  assert.match(current, /乙/);
  assert.doesNotMatch(current, /甲/);
});

test("取消存储维护保留已有日志和未完成行，暂停期间不落盘", () => {
  const file = tempLog();
  const log = createServiceLog(file);
  log.write("existing\npartial ");
  const before = fs.readFileSync(file);
  const lease = log.suspendForStorage();
  assert.equal(log.write("paused record\n"), false);
  assert.deepEqual(fs.readFileSync(file), before);
  assert.throws(() => log.suspendForStorage(), /already_paused/);
  lease.resume();
  log.write("continued\n");
  assert.match(fs.readFileSync(file, "utf8"), /partial continued/);
  assert.throws(() => lease.resume(), /pause_expired/);
});

test("清理后重建大小与行缓冲，不把旧内容写回或按旧大小轮换", () => {
  const file = tempLog();
  const log = createServiceLog(file, 80);
  log.write(`${"a".repeat(45)}\nold partial`);
  const lease = log.suspendForStorage();
  fs.unlinkSync(file);
  lease.rebase();
  lease.resume();
  log.write("new line\n");
  assert.match(fs.readFileSync(file, "utf8"), /new line/);
  assert.doesNotMatch(fs.readFileSync(file, "utf8"), /old partial|a{45}/);
  assert.equal(fs.existsSync(`${file}.1`), false);
});
