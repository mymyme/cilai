import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { verifyPublicArtifact } from './verify-public-artifact.mjs';
const bytes = Buffer.from('完整文件，按原始字节顺序验证 SHA-512');
const url = 'https://updates.example/package.zip';
const sha512 = createHash('sha512').update(bytes).digest('base64');
test('并行 Range 回读顺序不同仍校验整个文件，保留 Range 与固定编码请求头', async () => {
  const ranges = []; const progress = [];
  const result = await verifyPublicArtifact({url,size:bytes.length,blockSize:7,concurrency:3,onProgress:n=>progress.push(n),fetchImpl:async(_url,options)=>{
    assert.equal(options.headers.get('accept-encoding'),'identity');
    const range=options.headers.get('range');ranges.push(range);
    const [,a,b]=range.match(/^bytes=(\d+)-(\d+)$/);const start=Number(a),end=Number(b);
    await new Promise(done=>setTimeout(done,(20-start%20)));
    return new Response(bytes.subarray(start,end+1),{status:206,headers:{'content-range':`bytes ${start}-${end}/${bytes.length}`,'content-length':String(end-start+1)}});
  }});
  assert.equal(result.digest,sha512);assert.equal(result.size,bytes.length);assert.ok(ranges.length>3);assert.equal(progress.at(-1),bytes.length);
});
test('元数据完整读取；拒绝错误范围、长度、编码及截断正文',async()=>{
  const small=await verifyPublicArtifact({url,size:bytes.length,fetchImpl:async()=>new Response(bytes)});
  assert.equal(small.digest,sha512);
  for(const response of [()=>new Response(bytes),()=>new Response(bytes.subarray(0,7),{status:206,headers:{'content-range':'bytes 0-6/999'}}),
    ()=>new Response(bytes.subarray(0,1),{status:206,headers:{'content-range':`bytes 0-6/${bytes.length}`}}),
    ()=>new Response(bytes.subarray(0,7),{status:206,headers:{'content-range':`bytes 0-6/${bytes.length}`,'content-encoding':'gzip'}})]) {
    await assert.rejects(verifyPublicArtifact({url,size:bytes.length,blockSize:7,concurrency:1,fetchImpl:async()=>response()}));
  }
});
