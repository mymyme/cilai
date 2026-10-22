import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { writeUpdateConfiguration } from './update-configuration.mjs';

test('两端个人本地测试包只记录个人身份，移除更新元数据', async t => {
  const resources = await mkdtemp(join(tmpdir(), 'cilai-personal-package-'));
  t.after(() => rm(resources, { recursive: true, force: true }));
  for (const [platform, arch] of [['darwin', 'arm64'], ['darwin', 'x64'], ['win32', 'x64']]) {
    await writeFile(join(resources, 'app-update.yml'), 'provider: generic');
    await writeFile(join(resources, 'portable-update.json'), '{}');
    const configuration = await writeUpdateConfiguration({ resources, platform, arch, profile: 'debug', channel: 'testing', distribution: 'local' });
    assert.equal(configuration.applicationId, 'cn.cilai.personal.workbench');
    assert.equal(configuration.distribution, 'local');
    assert.deepEqual(configuration.channels, {});
    assert.deepEqual(JSON.parse(await readFile(join(resources, 'update-channels.json'), 'utf8')), configuration);
    await assert.rejects(readFile(join(resources, 'app-update.yml')), { code: 'ENOENT' });
    await assert.rejects(readFile(join(resources, 'portable-update.json')), { code: 'ENOENT' });
  }
});

test('未配置个人源时拒绝在线分发，保留原有文件', async t => {
  const resources = await mkdtemp(join(tmpdir(), 'cilai-personal-update-rejected-'));
  t.after(() => rm(resources, { recursive: true, force: true }));
  await writeFile(join(resources, 'update-channels.json'), '原有配置');
  for (const [platform, arch, distribution] of [['darwin', 'arm64', 'native'], ['win32', 'x64', 'native'], ['win32', 'x64', 'portable']]) {
    await assert.rejects(writeUpdateConfiguration({ resources, platform, arch, profile: 'release', channel: 'stable', distribution }), /个人版更新源尚未配置/);
    assert.equal(await readFile(join(resources, 'update-channels.json'), 'utf8'), '原有配置');
  }
});
