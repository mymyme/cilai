import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, readFile, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createPortableArchive, digestFile } from './portable-package.mjs';
test('Windows PowerShell 5.1 生成的实际 ZIP 使用正斜杠并保留中文名称', {skip:process.platform!=='win32'}, async()=>{
  const root=await mkdtemp(join(tmpdir(),'cilai-zip-contract-'));const bundle=join(root,'bundle');const output=join(root,'out');
  await mkdir(join(bundle,'resources'),{recursive:true});await mkdir(join(bundle,'locales'));await mkdir(output);
  await writeFile(join(bundle,'词来工作台.exe'),'fixture');await writeFile(join(bundle,'resources/app.asar'),'fixture');await writeFile(join(bundle,'locales/af.pak'),'fixture');
  const manifest=await createPortableArchive({bundle,output,version:'1.2.3'});
  const result=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',
    '[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); Add-Type -AssemblyName System.IO.Compression.FileSystem; $Archive=[IO.Compression.ZipFile]::OpenRead($env:CILAI_ZIP_CONTRACT); try { ConvertTo-Json -InputObject @($Archive.Entries | ForEach-Object { $_.FullName }) -Compress } finally { $Archive.Dispose() }'],
    {encoding:'utf8',windowsHide:true,env:{...process.env,CILAI_ZIP_CONTRACT:join(output,manifest.archive.file)}});
  assert.equal(result.status,0,result.stderr);const names=JSON.parse(result.stdout);
  assert.ok(names.includes('词来工作台.exe'));assert.ok(names.includes('locales/af.pak'));assert.ok(names.every(name=>!name.includes('\\')));
});

test('同版本本地重建原子替换完整 ZIP，不阻止开发构建和启动', {skip:process.platform!=='win32'}, async()=>{
  const root=await mkdtemp(join(tmpdir(),'cilai-zip-rebuild-'));const bundle=join(root,'bundle');const output=join(root,'out');
  await mkdir(join(bundle,'resources'),{recursive:true});await mkdir(output);
  await writeFile(join(bundle,'词来工作台.exe'),'first build');
  const first=await createPortableArchive({bundle,output,version:'1.2.3',channel:'testing',profile:'debug'});
  await writeFile(join(bundle,'词来工作台.exe'),'second build');
  const second=await createPortableArchive({bundle,output,version:'1.2.3',channel:'testing',profile:'debug'});
  assert.equal(second.archive.file,first.archive.file);assert.notEqual(second.archive.sha512,first.archive.sha512);
  assert.deepEqual(JSON.parse(await readFile(join(output,'portable.json'),'utf8')),second);
  const actual=await digestFile(join(output,second.archive.file),'sha512','base64');
  assert.equal(actual.digest,second.archive.sha512);assert.equal(actual.size,second.archive.size);
  assert.equal((await readdir(output)).some(name=>name.endsWith('.building')),false);
});
