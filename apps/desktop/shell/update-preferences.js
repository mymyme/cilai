const fs = require('node:fs');
const path = require('node:path');
const { randomUUID } = require('node:crypto');

function updatePreferences(userData) {
  const file = path.join(userData, 'update-preferences.json');
  return {
    load() {
      try {
        if (fs.statSync(file).size > 1024) throw new Error('更新偏好过大');
        const value = JSON.parse(fs.readFileSync(file, 'utf8'));
        if (value.schemaVersion !== 1 || typeof value.autoDownload !== 'boolean') throw new Error('更新偏好无效');
        if (value.pausedCandidate != null && !/^(stable|testing):\d+\.\d+\.\d+$/.test(value.pausedCandidate)) throw new Error('暂停版本无效');
        return { autoDownload: value.autoDownload, pausedCandidate: value.pausedCandidate ?? null };
      } catch (error) {
        if (error.code === 'ENOENT') return { autoDownload: true };
        throw new Error(`无法读取更新偏好：${error.message}`);
      }
    },
    save(value) {
      if (typeof value.autoDownload !== 'boolean') throw new Error('自动下载设置无效');
      fs.mkdirSync(userData, { recursive: true });
      const temporary = `${file}.${randomUUID()}.tmp`;
      try {
        fs.writeFileSync(temporary, JSON.stringify({ schemaVersion: 1, autoDownload: value.autoDownload, pausedCandidate: value.pausedCandidate ?? null }) + '\n', { flag: 'wx', mode: 0o600 });
        fs.renameSync(temporary, file);
      } catch (error) {
        try { fs.unlinkSync(temporary); } catch (cleanup) { if (cleanup.code !== 'ENOENT') throw cleanup; }
        throw new Error(`无法保存更新偏好：${error.message}`);
      }
    },
  };
}

module.exports = { updatePreferences };
