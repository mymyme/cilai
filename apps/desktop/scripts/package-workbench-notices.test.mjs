import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { installWorkbenchNotices, verifyWorkbenchNotices } from './package-workbench-notices.mjs';

const source = fileURLToPath(new URL('../../..', import.meta.url));
function fixture(context) {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'cilai-workbench-notices-'));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  const write = (name, value) => { const file = join(root, name); mkdirSync(dirname(file), { recursive: true }); writeFileSync(file, value); };
  for (const name of ['LICENSE', 'NOTICE', 'Cargo.toml', 'package.json', 'apps/desktop/package.json', 'apps/desktop-ui/package.json', 'packages/app-ui/package.json', 'packages/app-ui/LICENSE', 'packages/app-ui/NOTICE']) write(name, readFileSync(join(source, name)));
  return { root, write, desktopDir: join(root, 'apps/desktop'), resources: join(root, 'output/resources') };
}

test('实际源码中的标准第一方许可和共享组件声明完整随包，重复核对不改写', context => {
  const f = fixture(context);
  const receipt = installWorkbenchNotices(f);
  assert.equal(receipt.license, 'Apache-2.0');
  for (const name of ['LICENSE', 'NOTICE']) assert.deepEqual(readFileSync(join(f.resources, 'licenses/workbench', name)), readFileSync(join(f.root, name)));
  assert.deepEqual(verifyWorkbenchNotices(f), receipt);
  assert.deepEqual(installWorkbenchNotices(f), receipt);
  writeFileSync(join(f.resources, 'licenses/workbench/NOTICE'), 'changed notice');
  assert.throws(() => installWorkbenchNotices(f), /不符/);
  assert.equal(readFileSync(join(f.resources, 'licenses/workbench/NOTICE'), 'utf8'), 'changed notice');
});

for (const name of ['package.json', 'apps/desktop/package.json', 'apps/desktop-ui/package.json', 'packages/app-ui/package.json', 'Cargo.toml']) test(`第一方声明不符时拒绝生成测试包：${name}`, context => {
  const f = fixture(context);
  f.write(name, readFileSync(join(f.root, name), 'utf8').replace('Apache-2.0', 'LicenseRef-Proprietary'));
  assert.throws(() => installWorkbenchNotices(f), /声明不一致/);
  assert.equal(existsSync(join(f.resources, 'licenses/workbench')), false);
});

test('删减或修改标准许可、共享通知与链接来源均拒绝', context => {
  for (const kind of ['shortened', 'shared', 'missing', 'link']) {
    const f = fixture(context);
    if (kind === 'shortened') f.write('LICENSE', 'Apache-2.0');
    if (kind === 'shared') f.write('packages/app-ui/NOTICE', 'different');
    if (kind === 'missing') rmSync(join(f.root, 'LICENSE'));
    if (kind === 'link') { rmSync(join(f.root, 'NOTICE')); symlinkSync(join(f.root, 'LICENSE'), join(f.root, 'NOTICE')); }
    assert.throws(() => installWorkbenchNotices(f), /不符|不一致|缺少|链接/);
    assert.equal(existsSync(join(f.resources, 'licenses/workbench')), false);
  }
});

test('最终包缺失、修改或多出许可文件时拒绝，校验不覆盖产物', context => {
  const f = fixture(context);
  assert.throws(() => verifyWorkbenchNotices(f), /缺少/);
  installWorkbenchNotices(f);
  f.write('output/resources/licenses/workbench/LICENSE', 'shortened');
  assert.throws(() => verifyWorkbenchNotices(f), /不符/);
  f.write('output/resources/licenses/workbench/LICENSE', readFileSync(join(f.root, 'LICENSE')));
  f.write('output/resources/licenses/workbench/unregistered', 'extra');
  assert.throws(() => verifyWorkbenchNotices(f), /清单不符/);
});
