const fs = require('node:fs');
const path = require('node:path');
const { randomUUID } = require('node:crypto');
const { compareVersions } = require('./portable-update');

const CHANNELS = ['stable', 'testing'];
const { APPLICATION_ID } = require('./personal-isolation');
const nativePolicies = new WeakMap();

function validateChannel(channel) {
  if (!CHANNELS.includes(channel)) throw new Error('更新通道无效，只允许正式版或测试版');
  return channel;
}

function channelConfiguration({ platform, arch, distribution, buildChannel, buildProfile }) {
  if (!['win32', 'darwin'].includes(platform) || !['x64', 'arm64'].includes(arch)
    || (platform === 'win32' && arch !== 'x64')) throw new Error('更新平台或架构不受支持');
  if (distribution !== 'local') throw new Error('个人版更新源尚未配置，仅支持本地测试包。');
  if (!['debug', 'release'].includes(buildProfile)) throw new Error('更新构建配置无效');
  validateChannel(buildChannel);
  return { schemaVersion: 1, applicationId: APPLICATION_ID, platform, arch, distribution, buildChannel, buildProfile, channels: {} };
}

function validateConfiguration(value, platform, arch) {
  if (value?.schemaVersion !== 1 || value.applicationId !== APPLICATION_ID || value.platform !== platform || value.arch !== arch) throw new Error('更新配置与个人版身份或平台不匹配');
  const expected = channelConfiguration(value);
  if (!value.channels || typeof value.channels !== 'object' || Array.isArray(value.channels) || Object.keys(value.channels).length) throw new Error('个人版本地测试包不得包含更新源');
  return expected;
}

function loadUpdateConfiguration({ resourcesPath, platform, arch }) {
  const file = path.join(resourcesPath, 'update-channels.json');
  if (fs.statSync(file).size > 4096) throw new Error('更新配置过大');
  return validateConfiguration(JSON.parse(fs.readFileSync(file, 'utf8')), platform, arch);
}

function channelPreferences(userData) {
  const file = path.join(userData, 'update-channel.json');
  return {
    load(defaultChannel = 'stable') {
      validateChannel(defaultChannel);
      try {
        if (fs.statSync(file).size > 1024) throw new Error('更新通道偏好过大');
        const value = JSON.parse(fs.readFileSync(file, 'utf8'));
        if (value?.schemaVersion !== 1) throw new Error('更新通道偏好版本无效');
        return validateChannel(value.channel);
      } catch (error) {
        if (error.code === 'ENOENT') return defaultChannel;
        throw new Error(`无法读取更新通道：${error.message}`);
      }
    },
    save(channel) {
      validateChannel(channel);
      fs.mkdirSync(userData, { recursive: true });
      const temporary = `${file}.${randomUUID()}.tmp`;
      try {
        fs.writeFileSync(temporary, JSON.stringify({ schemaVersion: 1, channel }) + '\n', { flag: 'wx', mode: 0o600 });
        fs.renameSync(temporary, file);
      } catch (error) {
        try { fs.unlinkSync(temporary); } catch (cleanup) { if (cleanup.code !== 'ENOENT') throw cleanup; }
        throw new Error(`无法保存更新通道：${error.message}`);
      }
    },
  };
}

function configureChannel(updater, configuration, channel) {
  validateChannel(channel);
  if (configuration.distribution === 'local') throw new Error('本地测试包不支持在线更新');
  updater.setFeedURL({ provider: 'generic', url: configuration.channels[channel], channel: 'latest' });
  const switching = channel !== configuration.buildChannel;
  // A channel switch installs that channel's latest build, even if its version
  // is lower. Within the installed channel, versions still only move forward.
  updater.allowDowngrade = switching;
  updater.allowPrerelease = false;
  if (configuration.distribution === 'native') {
    if (typeof updater.isUpdateAvailable !== 'function') throw new Error('原生更新器不支持跨通道版本切换');
    if (!nativePolicies.has(updater)) {
      const original = updater.isUpdateAvailable.bind(updater);
      updater.isUpdateAvailable = async info => {
        const policy = nativePolicies.get(updater);
        const equal = compareVersions(info.version, updater.currentVersion.version) === 0;
        if (!equal || !policy.switching) return original(info);
        // electron-updater rejects equal versions before checking support or
        // rollout. Keep both gates when switching equal-version channel builds.
        return Boolean(await updater.isUpdateSupported(info))
          && Boolean(await updater.isUserWithinRollout(info));
      };
    }
    nativePolicies.set(updater, { switching });
  }
}

module.exports = { CHANNELS, validateChannel, channelConfiguration, validateConfiguration,
  loadUpdateConfiguration, channelPreferences, configureChannel };
