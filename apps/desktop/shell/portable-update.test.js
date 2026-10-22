const test = require('node:test');
const assert = require('node:assert/strict');
const { mkdtemp, readFile, writeFile, readdir, mkdir } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const path = require('node:path');
const { createHash, randomUUID } = require('node:crypto');
const { EventEmitter } = require('node:events');
const { PortableUpdater, validateManifest, compareVersions, checkedFetch } = require('./portable-update');
const systemRoot = process.env.SystemRoot;
test.before(() => { process.env.SystemRoot = 'C:\\Windows'; });
test.after(() => {
  if (systemRoot === undefined) delete process.env.SystemRoot;
  else process.env.SystemRoot = systemRoot;
});
const feed = 'https://updates.example/workbench/portable.json';
const bytes = Buffer.from('portable zip fixture');
const manifest = () => ({ schemaVersion: 1, kind: 'cilai-portable', platform: 'win32', arch: 'x64', version: '1.2.3',
  archive: { file: 'cilai-workbench-1.2.3-win-x64-portable.zip', size: bytes.length, sha512: createHash('sha512').update(bytes).digest('base64') },
  packageManifestSha256: 'a'.repeat(64), releaseNotes: '中文更新说明' });
async function fixture(fetchImpl, extra = {}) {
  const root = await mkdtemp(path.join(tmpdir(), 'cilai-updater-test-'));
  const app = { quits: 0, quit() { this.quits++; } };
  return new PortableUpdater({ config: { feed }, currentVersion: '1.0.0', cacheRoot: root, executable: path.join(root, 'app', '词来工作台.exe'),
    app, fetchImpl: fetchImpl ?? (async (url) => url.endsWith('.json') ? Response.json(manifest()) : new Response(bytes)), ...extra });
}
test('版本比较拒绝非三段数字版本', () => {
  assert.equal(compareVersions('1.10.0', '1.9.99'), 1);
  assert.equal(compareVersions('1.2.3', '1.2.3'), 0);
  assert.throws(() => compareVersions('1.2.3-beta', '1.0.0'));
});
test('拒绝错误平台、任意文件路径、超大包和无效摘要', () => {
  for (const mutate of [m => {m.platform='darwin';}, m => {m.archive.file='../evil.zip';},
    m => {m.archive.size=3*1024**3;}, m => {m.archive.sha512='bad';}, m => {m.packageManifestSha256='bad';}]) {
    const value=manifest(); mutate(value); assert.throws(() => validateManifest(value,feed));
  }
  assert.throws(() => validateManifest(manifest(),'http://updates.example/portable.json'));
});
test('仅允许同源 HTTPS 重定向', async () => {
  await assert.rejects(checkedFetch(feed, async()=>new Response(null,{status:302,headers:{location:'https://evil.example/a'}})), /来源不匹配/);
  await assert.rejects(checkedFetch(feed, async()=>new Response(null,{status:302,headers:{location:'http://updates.example/a'}})), /来源不匹配/);
  let calls=0;
  const result=await checkedFetch(feed,async()=>++calls===1?new Response(null,{status:302,headers:{location:'/new.json'}}):Response.json({ok:true}));
  assert.equal((await result.json()).ok,true);
});
test('分片 UTF8 元数据完整解码，限制清单大小', async () => {
  const data=Buffer.from(JSON.stringify(manifest()));
  const updater=await fixture(async()=>new Response(new ReadableStream({start(controller){for(const byte of data)controller.enqueue(Uint8Array.of(byte));controller.close();}})));
  const result=await updater.checkForUpdates(); assert.equal(result.releaseNotes,'中文更新说明');
  const large=await fixture(async()=>new Response('x'.repeat(65537)));
  await assert.rejects(large.checkForUpdates(),/元数据过大/);
});
test('当前或更旧版本不提供更新',async()=>{
  for(const version of ['1.2.3','2.0.0']) {
    const updater=await fixture(null,{currentVersion:version});
    assert.equal(await updater.checkForUpdates(),null);
  }
});
test('下载流校验完整性，产生进度并复用已验证缓存',async()=>{
  let downloads=0;
  const updater=await fixture(async(url)=>url.endsWith('.json')?Response.json(manifest()):(downloads++,new Response(bytes)));
  const progress=[]; updater.on('download-progress',p=>progress.push(p));
  await updater.checkForUpdates(); const [file]=await updater.downloadUpdate();
  assert.deepEqual(await readFile(file),bytes); assert.equal(progress.at(-1).percent,100);
  await updater.downloadUpdate(); assert.equal(downloads,1);
  await writeFile(file,'corrupt'); await updater.downloadUpdate(); assert.equal(downloads,2);
});
test('损坏包或断流不产生可安装缓存，允许重新下载',async()=>{
  for(const response of [()=>new Response(Buffer.alloc(bytes.length)),()=>new Response(bytes.subarray(0,2)),
    ()=>new Response(bytes,{headers:{'content-length':'999'}}),
    ()=>new Response(new ReadableStream({start(c){c.enqueue(bytes.subarray(0,2));c.error(new Error('断流'));}}))]) {
    const updater=await fixture(async(url)=>url.endsWith('.json')?Response.json(manifest()):response());
    await updater.checkForUpdates(); await assert.rejects(updater.downloadUpdate());
    assert.ok((await readdir(updater.cacheRoot)).every(file => file.endsWith('.part'))); assert.equal(updater.downloaded,null);
  }
});
test('断流保留前缀，重新启动后按 Range 续传并核验完整摘要', async () => {
  const updater = await fixture(async url => url.endsWith('.json') ? Response.json(manifest()) : new Response(bytes.subarray(0, 5)));
  await updater.checkForUpdates();
  await assert.rejects(updater.downloadUpdate(), /中断/);
  assert.deepEqual(await readFile(updater.cacheFile() + '.part'), bytes.subarray(0, 5));
  let downloads = 0;
  const resumed = await fixture(async (url, options) => {
    if (url.endsWith('.json')) return Response.json(manifest());
    downloads++;
    assert.equal(options.headers.get('range'), 'bytes=5-');
    return new Response(bytes.subarray(5), { status: 206, headers: {
      'Content-Range': `bytes 5-${bytes.length - 1}/${bytes.length}`, 'Content-Length': String(bytes.length - 5),
    } });
  }, { cacheRoot: updater.cacheRoot });
  await resumed.checkForUpdates();
  const [file] = await resumed.downloadUpdate();
  assert.deepEqual(await readFile(file), bytes);
  await resumed.checkForUpdates();
  assert.equal(await resumed.restoreDownloadedUpdate(), true);
  assert.equal(downloads, 1);
});
test('服务端不支持 Range 时完整重取，错误范围或损坏前缀不能安装', async () => {
  for (const mode of ['full', 'bad-range', 'bad-prefix']) {
    const updater = await fixture(async (url, options) => {
      if (url.endsWith('.json')) return Response.json(manifest());
      assert.equal(options.headers.get('range'), 'bytes=5-');
      return mode === 'full' ? new Response(bytes) : new Response(bytes.subarray(5), { status: 206,
        headers: { 'Content-Range': `bytes ${mode === 'bad-range' ? 4 : 5}-${bytes.length - 1}/${bytes.length}` } });
    });
    await updater.checkForUpdates();
    await writeFile(updater.cacheFile() + '.part', mode === 'bad-prefix' ? Buffer.alloc(5) : bytes.subarray(0, 5));
    if (mode === 'full') {
      const [file] = await updater.downloadUpdate(); assert.deepEqual(await readFile(file), bytes);
    } else {
      await assert.rejects(updater.downloadUpdate(), /范围不匹配|校验失败/);
      assert.equal(updater.downloaded, null);
      assert.deepEqual(await readdir(updater.cacheRoot), []);
    }
  }
});
test('安装前再次检查缓存，损坏后不退出当前应用',async()=>{
  const updater=await fixture(); await updater.checkForUpdates(); const [file]=await updater.downloadUpdate();
  await writeFile(file,'changed'); await assert.rejects(updater.quitAndInstall(),/缓存已变化/); assert.equal(updater.app.quits,0);
});
test('程序被改名、置于磁盘根目录或内含用户数据时不退出应用',async()=>{
  for (const kind of ['renamed','root','profile-inside']) {
    const updater=await fixture(); await updater.checkForUpdates(); await updater.downloadUpdate();
    if(kind==='renamed')updater.executable=path.join(path.dirname(updater.executable),'renamed.exe');
    if(kind==='root')updater.executable=path.join(path.parse(updater.executable).root,'词来工作台.exe');
    if(kind==='profile-inside')updater.cacheRoot=path.join(path.dirname(updater.executable),'profile','portable-updates');
    await assert.rejects(updater.quitAndInstall(),/原始文件名|独立文件夹/);
    assert.equal(updater.app.quits,0);
  }
});
test('异步启动更新助手成功后退出，助手使用 UTF8 BOM 和固定参数',async()=>{
  let invocation;
  const updater=await fixture(null,{launch:(...args)=>{
    invocation=args; const child=new EventEmitter(); child.unref=()=>{}; child.kill=()=>{};
    queueMicrotask(async()=>{
      try {
        const input=args[2].env.CILAI_UPDATE_HANDOFF;
        const handoff=JSON.parse(await readFile(input,'utf8'));
        await writeFile(path.join(path.dirname(input),'started.json'),JSON.stringify({id:handoff.id,nonce:handoff.nonce,version:handoff.version}));
        child.emit('spawn');
      }catch(error){child.emit('error',error);}
    }); return child;
  }});
  await updater.checkForUpdates(); await updater.downloadUpdate(); await updater.quitAndInstall();
  assert.equal(updater.app.quits,1); assert.equal(invocation[2].windowsHide,true);
  const helper=invocation[2].env.CILAI_UPDATE_HELPER;
  assert.deepEqual([...((await readFile(helper)).subarray(0,3))],[239,187,191]);
  const handoff=JSON.parse(await readFile(path.join(path.dirname(helper),'handoff.json'),'utf8'));
  assert.equal(handoff.version,'1.2.3'); assert.equal(handoff.target,path.dirname(updater.executable));
});
test('助手未运行或确认不匹配时保留当前应用并停止助手',async()=>{
  for(const kind of ['timeout','mismatch']) {
    let stopped=false;
    const updater=await fixture(null,{helperReadyTimeoutMs:30,launch:(_command,args,options)=>{
      const child=new EventEmitter();child.unref=()=>{};child.kill=()=>{stopped=true;};
      queueMicrotask(async()=>{
        try {
          if(kind==='mismatch') {
            const input=options.env.CILAI_UPDATE_HANDOFF;
            await writeFile(path.join(path.dirname(input),'started.json'),JSON.stringify({id:'wrong',nonce:'wrong',version:'9.0.0'}));
          }
          child.emit('spawn');
        }catch(error){child.emit('error',error);}
      });return child;
    }});
    await updater.checkForUpdates();await updater.downloadUpdate();
    await assert.rejects(updater.quitAndInstall(),/启动超时|确认不匹配/);
    assert.equal(updater.app.quits,0);assert.equal(stopped,true);
  }
});
test('启动确认绑定本次更新、目标目录和版本',async()=>{
  const updater=await fixture(null,{currentVersion:'1.2.3'}); const id=randomUUID();
  const ticket=path.join(updater.cacheRoot,id); await mkdir(ticket);
  const handoff={id,nonce:randomUUID(),version:'1.2.3',target:path.dirname(updater.executable)};
  await writeFile(path.join(ticket,'handoff.json'),JSON.stringify(handoff));
  await updater.acknowledgeStartup([`--cilai-update=${id}`]);
  assert.equal(JSON.parse(await readFile(path.join(ticket,'ready.json'),'utf8')).nonce,handoff.nonce);
  handoff.version='2.0.0'; await writeFile(path.join(ticket,'handoff.json'),JSON.stringify(handoff));
  await assert.rejects(updater.acknowledgeStartup([`--cilai-update=${id}`]),/范围不匹配/);
});
