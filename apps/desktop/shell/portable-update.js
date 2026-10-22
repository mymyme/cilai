const { EventEmitter } = require('node:events');
const { createHash, randomUUID } = require('node:crypto');
const { createReadStream, createWriteStream, existsSync, readFileSync } = require('node:fs');
const fs = require('node:fs/promises');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { Readable, Transform } = require('node:stream');
const { pipeline } = require('node:stream/promises');
const { portableHelperInvocation } = require('./windows-update-launcher');

const KIND = 'cilai-portable';
const MAX_ARCHIVE = 2 * 1024 * 1024 * 1024;
const TICKET = /^[a-f0-9-]{36}$/;

function versionParts(value) {
  if (typeof value !== 'string' || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(value)) throw new Error('更新版本格式无效');
  const parts = value.split('.').map(Number);
  if (parts.some((part) => !Number.isSafeInteger(part))) throw new Error('更新版本超出范围');
  return parts;
}

function compareVersions(left, right) {
  const a = versionParts(left); const b = versionParts(right);
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return Math.sign(a[i] - b[i]);
  return 0;
}

function feedUrl(value) {
  const url = new URL(value);
  if (url.protocol !== 'https:' || url.username || url.password || url.hash || url.search) throw new Error('免安装更新源必须使用固定 HTTPS 地址');
  return url;
}

function validateManifest(value, feed, expectedChannel = null) {
  if (value?.schemaVersion !== 1 || value.kind !== KIND || value.platform !== 'win32' || value.arch !== 'x64') throw new Error('更新包类型或平台不匹配');
  versionParts(value.version);
  if (value.channel !== undefined && !['stable', 'testing'].includes(value.channel)) throw new Error('更新清单通道无效');
  if (expectedChannel && (value.channel ?? 'stable') !== expectedChannel) throw new Error('更新包与所选通道不匹配');
  const expected = `cilai-workbench-${value.version}-win-x64-${value.channel === 'testing' ? 'testing-' : ''}portable.zip`;
  const archive = value.archive;
  if (!archive || archive.file !== expected || !Number.isSafeInteger(archive.size) || archive.size <= 0 || archive.size > MAX_ARCHIVE
    || !/^[A-Za-z0-9+/]{86}==$/.test(archive.sha512 ?? '') || !/^[a-f0-9]{64}$/.test(value.packageManifestSha256 ?? '')) throw new Error('更新包清单无效');
  const source = feedUrl(feed);
  const url = new URL(archive.file, source);
  if (url.origin !== source.origin) throw new Error('更新包来源不匹配');
  return { ...value, archive: { ...archive, url: url.href } };
}

async function checkedFetch(url, fetchImpl, options = {}) {
  const origin = new URL(url).origin;
  let target = new URL(url);
  for (let redirects = 0; redirects <= 3; redirects++) {
    const headers = new Headers(options.headers);
    headers.set('Accept-Encoding', 'identity');
    const response = await fetchImpl(target.href, { ...options, redirect: 'manual', cache: 'no-store', headers });
    if ([301, 302, 303, 307, 308].includes(response.status)) {
      const next = new URL(response.headers.get('location'), target);
      await response.body?.cancel();
      if (next.protocol !== 'https:' || next.origin !== origin || next.username || next.password) throw new Error('更新下载重定向来源不匹配');
      target = next;
      continue;
    }
    if (!response.ok) throw new Error(`更新源请求失败（HTTP ${response.status}）`);
    return response;
  }
  throw new Error('更新源重定向次数过多');
}

async function fileSha512(file) {
  const hash = createHash('sha512'); let size = 0;
  for await (const bytes of createReadStream(file)) { hash.update(bytes); size += bytes.length; }
  return { sha512: hash.digest('base64'), size };
}

class PortableUpdater extends EventEmitter {
  constructor({ config, currentVersion, executable, cacheRoot, app, fetchImpl = fetch, launch = spawn, helperReadyTimeoutMs = 15000 }) {
    super();
    this.feed = feedUrl(config.feed).href;
    this.currentVersion = currentVersion;
    versionParts(currentVersion);
    this.executable = executable;
    this.cacheRoot = cacheRoot;
    this.app = app;
    this.fetch = fetchImpl;
    this.launch = launch;
    this.helperReadyTimeoutMs = helperReadyTimeoutMs;
    this.distribution = 'portable';
    this.latest = null;
    this.downloaded = null;
  }

  async checkForUpdates() {
    this.latest = null;
    this.downloaded = null;
    this.emit('checking-for-update');
    const response = await checkedFetch(this.feed, this.fetch, { signal: AbortSignal.timeout(20000) });
    const chunks = []; let size = 0;
    for await (const bytes of response.body) {
      size += bytes.length;
      if (size > 65536) throw new Error('更新元数据过大');
      chunks.push(Buffer.from(bytes));
    }
    const manifest = validateManifest(JSON.parse(Buffer.concat(chunks).toString('utf8')), this.feed,
      new URL(this.feed).pathname.includes('/testing/') ? 'testing' : 'stable');
    if (compareVersions(manifest.version, this.currentVersion) <= 0 && !this.allowDowngrade) {
      this.latest = null;
      this.emit('update-not-available', { version: manifest.version, cilaiChannel: manifest.channel ?? 'stable' });
      return null;
    }
    this.latest = manifest;
    this.emit('update-available', { version: manifest.version, cilaiChannel: manifest.channel ?? 'stable', releaseName: manifest.releaseName ?? null, releaseNotes: manifest.releaseNotes ?? null });
    return manifest;
  }

  setFeedURL({ url }) {
    this.feed = feedUrl(url).href;
    this.latest = null;
    this.downloaded = null;
  }

  cacheFile(manifest = this.latest) {
    return path.join(this.cacheRoot, `${manifest.version}-${manifest.archive.sha512.replace(/[^a-z0-9]/gi, '').slice(0, 24)}.zip`);
  }

  async restoreDownloadedUpdate() {
    if (!this.latest) return false;
    const manifest = this.latest;
    const file = this.cacheFile(manifest);
    if (!existsSync(file)) return false;
    const cached = await fileSha512(file);
    if (cached.sha512 !== manifest.archive.sha512 || cached.size !== manifest.archive.size) {
      await fs.unlink(file);
      throw new Error('更新缓存校验失败，请重新下载');
    }
    this.downloaded = { file, manifest };
    this.emit('update-downloaded', { version: manifest.version, cilaiChannel: manifest.channel ?? 'stable' });
    return true;
  }

  async downloadUpdate(cancellationToken) {
    if (!this.latest) throw new Error('当前没有可下载的更新');
    const manifest = this.latest;
    await fs.mkdir(this.cacheRoot, { recursive: true });
    const file = this.cacheFile(manifest);
    if (existsSync(file)) {
      const cached = await fileSha512(file);
      if (cached.sha512 === manifest.archive.sha512 && cached.size === manifest.archive.size) {
        this.downloaded = { file, manifest };
        this.emit('update-downloaded', { version: manifest.version, cilaiChannel: manifest.channel ?? 'stable' });
        return [file];
      }
      await fs.unlink(file);
    }
    const partial = `${file}.part`;
    const abort = new AbortController();
    const cancel = () => abort.abort(new Error('更新下载已暂停'));
    cancellationToken?.on('cancel', cancel);
    if (cancellationToken?.cancelled) cancel();
    let discard = false;
    try {
      let offset = await fs.stat(partial).then(stat => stat.size).catch(error => { if (error.code === 'ENOENT') return 0; throw error; });
      if (offset >= manifest.archive.size) {
        const cached = await fileSha512(partial);
        if (cached.size === manifest.archive.size && cached.sha512 === manifest.archive.sha512) {
          await fs.rename(partial, file);
          this.downloaded = { file, manifest };
          this.emit('update-downloaded', { version: manifest.version, cilaiChannel: manifest.channel ?? 'stable' });
          return [file];
        }
        await fs.unlink(partial);
        throw new Error('更新暂存文件校验失败，请重新下载');
      }
      abort.signal.throwIfAborted();
      const response = await checkedFetch(manifest.archive.url, this.fetch, {
        signal: AbortSignal.any([abort.signal, AbortSignal.timeout(60 * 60 * 1000)]),
        headers: offset ? { Range: `bytes=${offset}-` } : {},
      });
      const rejectResponse = async message => { discard = true; await response.body?.cancel(); throw new Error(message); };
      if (response.status === 206) {
        const range = response.headers.get('content-range');
        if (range !== `bytes ${offset}-${manifest.archive.size - 1}/${manifest.archive.size}`) await rejectResponse('更新续传范围不匹配');
      } else if (response.status === 200) {
        // A server may legally ignore Range. Start the verified full response
        // at zero instead of appending it to the existing prefix.
        offset = 0;
      } else await rejectResponse('更新下载响应无效');
      const length = response.headers.get('content-length');
      if (length !== null && Number(length) !== manifest.archive.size - offset) await rejectResponse('更新文件长度不匹配');
      if (response.headers.get('content-encoding') && response.headers.get('content-encoding') !== 'identity') await rejectResponse('更新文件传输编码不匹配');
      let transferred = offset; let lastProgress = 0;
      const hash = createHash('sha512'); const started = Date.now();
      if (offset) for await (const bytes of createReadStream(partial)) hash.update(bytes);
      const progress = new Transform({ transform: (bytes, _encoding, done) => {
        transferred += bytes.length;
        if (transferred > manifest.archive.size) { discard = true; done(new Error('更新文件超过声明长度')); return; }
        hash.update(bytes);
        if (Date.now() - lastProgress > 150 || transferred === manifest.archive.size) {
          lastProgress = Date.now();
          this.emit('download-progress', { transferred, total: manifest.archive.size, percent: transferred / manifest.archive.size * 100,
            bytesPerSecond: (transferred - offset) / Math.max((Date.now() - started) / 1000, 0.001) });
        }
        done(null, bytes);
      } });
      await pipeline(Readable.fromWeb(response.body), progress, createWriteStream(partial, { flags: offset ? 'a' : 'w', mode: 0o600 }), { signal: abort.signal });
      if (transferred !== manifest.archive.size) throw new Error('更新下载中断，可继续下载');
      if (hash.digest('base64') !== manifest.archive.sha512) { discard = true; throw new Error('更新文件校验失败，请重新下载'); }
      await fs.rename(partial, file);
      this.downloaded = { file, manifest };
      this.emit('update-downloaded', { version: manifest.version, cilaiChannel: manifest.channel ?? 'stable' });
      return [file];
    } catch (error) {
      if (discard) await fs.unlink(partial).catch((failure) => { if (failure.code !== 'ENOENT') throw failure; });
      throw error;
    } finally {
      cancellationToken?.removeListener('cancel', cancel);
    }
  }

  async quitAndInstall() {
    if (!this.downloaded) throw new Error('更新尚未下载完成');
    const target = path.dirname(this.executable);
    const relativeProfile = path.relative(target, path.dirname(this.cacheRoot));
    if (path.basename(this.executable) !== '词来工作台.exe') throw new Error('请保留程序原始文件名“词来工作台.exe”后重试更新');
    if (path.resolve(target).toLowerCase() === path.parse(path.resolve(target)).root.toLowerCase()
      || (relativeProfile !== '..' && !relativeProfile.startsWith('..' + path.sep) && !path.isAbsolute(relativeProfile))) {
      throw new Error('请把程序放在独立文件夹中，并将用户数据保存在程序目录之外后重试');
    }
    const { file, manifest } = this.downloaded;
    const actual = await fileSha512(file);
    if (actual.sha512 !== manifest.archive.sha512 || actual.size !== manifest.archive.size) throw new Error('更新缓存已变化，请重新下载');
    const probe = path.join(path.dirname(path.dirname(this.executable)), `.cilai-write-${randomUUID()}`);
    try { await fs.writeFile(probe, '', { flag: 'wx' }); await fs.unlink(probe); }
    catch (error) { throw new Error(`程序所在目录不可更新，请移到可写目录后重试：${error.message}`); }
    const id = randomUUID();
    const ticket = path.join(this.cacheRoot, id);
    await fs.mkdir(ticket);
    const helper = path.join(ticket, 'apply-portable-update.ps1');
    const helperText = await fs.readFile(path.join(__dirname, 'apply-portable-update.ps1'), 'utf8');
    await fs.writeFile(helper, '\ufeff' + helperText.replace(/^\ufeff/, ''), { flag: 'wx', mode: 0o600 });
    const handoff = { schemaVersion: 1, id, nonce: randomUUID(), target,
      archive: file, sha512: manifest.archive.sha512, size: manifest.archive.size, packageManifestSha256: manifest.packageManifestSha256,
      version: manifest.version, executableName: path.basename(this.executable), oldPid: process.pid, ticket, userData: path.dirname(this.cacheRoot) };
    const input = path.join(ticket, 'handoff.json');
    await fs.writeFile(input, JSON.stringify(handoff), { flag: 'wx', mode: 0o600 });
    const invocation = portableHelperInvocation(helper, input);
    let helperProcess;
    await new Promise((resolve, reject) => {
      const child = this.launch(invocation.command, invocation.args, invocation.options);
      helperProcess = child;
      child.once('error', reject);
      child.once('spawn', () => { child.unref(); resolve(); });
    });
    try {
      const deadline = Date.now() + this.helperReadyTimeoutMs;
      let started = false;
      do {
        const result = await fs.readFile(path.join(ticket, 'result.json'), 'utf8').then(JSON.parse).catch((error) => { if (error.code === 'ENOENT') return null; throw error; });
        if (result && result.status !== 'updated') throw new Error(`更新准备失败：${result.message}`);
        const ready = await fs.readFile(path.join(ticket, 'started.json'), 'utf8').then(JSON.parse).catch((error) => { if (error.code === 'ENOENT') return null; throw error; });
        if (ready) {
          if (ready.id !== id || ready.nonce !== handoff.nonce || ready.version !== handoff.version) throw new Error('更新助手启动确认不匹配');
          started = true;
          break;
        }
        if (Number.isInteger(helperProcess.exitCode) && helperProcess.exitCode !== 0) throw new Error(`更新助手未能运行（退出码 ${helperProcess.exitCode}），请检查系统脚本执行策略`);
        await new Promise((resolve) => setTimeout(resolve, 100));
      } while (Date.now() < deadline);
      if (!started) throw new Error('更新助手启动超时，请检查系统脚本执行策略后重试');
    } catch (error) {
      helperProcess.kill();
      throw error;
    }
    this.app.quit();
  }

  async acknowledgeStartup(args = process.argv) {
    const argument = args.find((value) => value.startsWith('--cilai-update='));
    if (!argument) return;
    const id = argument.slice('--cilai-update='.length);
    if (!TICKET.test(id)) throw new Error('更新确认编号无效');
    const ticket = path.join(this.cacheRoot, id);
    const handoff = JSON.parse(await fs.readFile(path.join(ticket, 'handoff.json'), 'utf8'));
    if (handoff.id !== id || handoff.version !== this.currentVersion || path.resolve(handoff.target).toLowerCase() !== path.dirname(this.executable).toLowerCase()) throw new Error('更新确认范围不匹配');
    const temporary = path.join(ticket, 'ready.tmp');
    await fs.writeFile(temporary, JSON.stringify({ id, nonce: handoff.nonce, version: this.currentVersion, pid: process.pid }), { flag: 'wx' });
    await fs.rename(temporary, path.join(ticket, 'ready.json'));
  }

  async previousFailure(args = process.argv) {
    const argument = args.find((value) => value.startsWith('--cilai-update-rollback='));
    if (!argument) return null;
    const id = argument.slice('--cilai-update-rollback='.length);
    if (!TICKET.test(id)) throw new Error('更新恢复编号无效');
    const result = JSON.parse(await fs.readFile(path.join(this.cacheRoot, id, 'result.json'), 'utf8'));
    if (path.resolve(result.target).toLowerCase() !== path.dirname(this.executable).toLowerCase()) throw new Error('更新恢复目录不匹配');
    return `更新未完成，已恢复原版本：${result.message}`;
  }
}

function loadPortableUpdater({ resourcesPath, platform, ...options }) {
  if (platform !== 'win32') return null;
  const file = path.join(resourcesPath, 'portable-update.json');
  if (!existsSync(file)) return null;
  const config = JSON.parse(readFileSync(file, 'utf8'));
  if (config.schemaVersion !== 1 || config.kind !== KIND || config.platform !== 'win32' || config.arch !== 'x64') throw new Error('免安装更新配置无效');
  return new PortableUpdater({ ...options, config });
}

module.exports = { PortableUpdater, loadPortableUpdater, validateManifest, compareVersions, checkedFetch, fileSha512 };
