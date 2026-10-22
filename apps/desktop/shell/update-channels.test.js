const test = require('node:test');
const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const { channelConfiguration, validateConfiguration, configureChannel } = require('./update-channels');
const { WorkbenchUpdater } = require('./updater');

for (const [platform, arch] of [['darwin', 'arm64'], ['darwin', 'x64'], ['win32', 'x64']]) {
  test(`${platform}/${arch} 本地包身份独立，更新控制器不联网`, async () => {
    const config = channelConfiguration({ platform, arch, distribution: 'local', buildChannel: 'testing', buildProfile: 'debug' });
    assert.deepEqual(validateConfiguration(config, platform, arch), config);
    assert.equal(config.applicationId, 'cn.cilai.personal.workbench');
    assert.deepEqual(config.channels, {});
    const updater = new EventEmitter();
    updater.checkForUpdates = () => { throw new Error('未配置更新源不得请求网络'); };
    const controller = new WorkbenchUpdater({ autoUpdater: updater, currentVersion: '0.1.0', enabled: true, configuration: config, broadcast() {} });
    await controller.start(); await controller.check('settings');
    assert.equal(controller.snapshot().supported, false);
    assert.throws(() => configureChannel(updater, config, 'stable'), /不支持在线更新/);
    controller.stop();
  });
}
test('企业更新身份、企业源和未登记的在线通道全部被拒绝', () => {
  const config = channelConfiguration({platform:'darwin',arch:'arm64',distribution:'local',buildChannel:'testing',buildProfile:'debug'});
  assert.throws(() => validateConfiguration({...config,applicationId:'com.cilai.workbench'}, 'darwin','arm64'), /身份或平台/);
  assert.throws(() => validateConfiguration({...config,channels:{stable:'https://updates.cilai.com.cn/workbench'}},'darwin','arm64'), /不得包含更新源/);
  for (const distribution of ['native','portable']) assert.throws(() => channelConfiguration({...config,distribution}), /更新源尚未配置/);
});
