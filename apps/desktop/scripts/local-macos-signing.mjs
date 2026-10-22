import { spawnSync } from 'node:child_process';

export function validDevelopmentIdentities(output) {
  return String(output).split(/\r?\n/).flatMap(line => {
    const match = /^\s*\d+\)\s+[A-Fa-f0-9]{40}\s+"(Apple Development: [^"\r\n]+)"\s*$/.exec(line);
    return match ? [match[1]] : [];
  });
}

export function readDevelopmentIdentities(run = spawnSync) {
  const result = run('/usr/bin/security', ['find-identity', '-v', '-p', 'codesigning'], { encoding: 'utf8' });
  if (result.error || result.status !== 0) throw new Error('无法检查本机有效开发签名身份');
  return validDevelopmentIdentities(result.stdout);
}

export function localMacSigningArguments({ platform, localTest, developmentIdentity, availableIdentities = [] }) {
  let identity = developmentIdentity?.trim();
  if (identity && (platform !== 'darwin' || !localTest)) throw new Error('开发签名身份仅用于 macOS 本地测试包');
  if (!localTest) return [];
  if (platform !== 'darwin') throw new Error('本地测试签名仅支持 macOS');
  const valid = availableIdentities.filter(value => typeof value === 'string' && value.startsWith('Apple Development: '));
  if (identity) {
    if (!identity.startsWith('Apple Development: ') || valid.filter(value => value === identity).length !== 1) {
      throw new Error('指定的开发签名身份无效、不在本机有效身份列表或存在多义');
    }
  } else {
    if (valid.length !== 1) throw new Error('本地测试包需要唯一有效 Apple Development 签名身份；请配置 CILAI_MAC_DEVELOPMENT_SIGNING_IDENTITY');
    [identity] = valid;
  }
  return ['--dir', '--config.forceCodeSigning=true', `--config.mac.identity=${identity}`, '--config.mac.type=development',
    '--config.mac.hardenedRuntime=false', '--config.mac.notarize=false', '--config.mac.timestamp=none',
    '--config.mac.entitlements=build/entitlements.mac.plist', '--config.mac.entitlementsInherit=build/entitlements.mac.inherit.plist'];
}
