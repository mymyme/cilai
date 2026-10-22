import assert from 'node:assert/strict';
import test from 'node:test';
import { localMacSigningArguments, readDevelopmentIdentities, validDevelopmentIdentities } from './local-macos-signing.mjs';
const identity = 'Apple Development: Synthetic Developer (TESTTEAM)';
const another = 'Apple Development: Other Developer (OTHERTEAM)';
const input = { platform: 'darwin', localTest: true };

test('唯一有效开发身份自动签名，本地包不请求公证且不能临时签名', () => {
  const args = localMacSigningArguments({ ...input, availableIdentities: [identity] });
  for (const value of ['--dir', '--config.forceCodeSigning=true', `--config.mac.identity=${identity}`, '--config.mac.type=development', '--config.mac.notarize=false', '--config.mac.timestamp=none', '--config.mac.entitlementsInherit=build/entitlements.mac.inherit.plist']) assert.ok(args.includes(value));
  assert.ok(!args.includes('--config.mac.identity=-'));
  assert.deepEqual(localMacSigningArguments({ platform: 'darwin', localTest: false }), []);
});

test('显式身份必须有效且唯一，不能猜测或选用正式发布身份', () => {
  assert.ok(localMacSigningArguments({ ...input, developmentIdentity: identity, availableIdentities: [identity, another] }).includes(`--config.mac.identity=${identity}`));
  for (const availableIdentities of [[], [another], [identity, identity]]) assert.throws(() => localMacSigningArguments({ ...input, developmentIdentity: identity, availableIdentities }), /签名/);
  for (const availableIdentities of [[], [identity, another]]) assert.throws(() => localMacSigningArguments({ ...input, availableIdentities }), /签名/);
  for (const value of ['-', 'Developer ID Application: Synthetic']) assert.throws(() => localMacSigningArguments({ ...input, developmentIdentity: value, availableIdentities: [value] }), /签名/);
  for (const value of [{ platform: 'darwin', localTest: false }, { platform: 'win32', localTest: true }]) assert.throws(() => localMacSigningArguments({ ...value, developmentIdentity: identity }), /签名/);
  assert.deepEqual(localMacSigningArguments({ platform: 'win32', localTest: false }), []);
});

test('系统只检查有效codesigning身份，忽略无效证书和非开发身份', () => {
  const digest = 'A'.repeat(40);
  const output = `  1) ${digest} "${identity}"\n  2) ${digest} "Developer ID Application: Example"\n  3) ${digest} "${another}" (CSSMERR_TP_CERT_EXPIRED)\n  1 valid identities found\n`;
  assert.deepEqual(validDevelopmentIdentities(output), [identity]);
  assert.deepEqual(readDevelopmentIdentities((program, args) => {
    assert.equal(program, '/usr/bin/security'); assert.deepEqual(args, ['find-identity', '-v', '-p', 'codesigning']);
    return { status: 0, stdout: output };
  }), [identity]);
  assert.throws(() => readDevelopmentIdentities(() => ({ status: 1, stdout: output })), /无法检查/);
  assert.throws(() => readDevelopmentIdentities(() => ({ status: null, error: new Error('denied') })), /无法检查/);
});
