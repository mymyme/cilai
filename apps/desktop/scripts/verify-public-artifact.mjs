import { createHash } from 'node:crypto';
import portable from '../shell/portable-update.js';

// 分段并行取回全部字节，按文件顺序计算同一个 SHA-512；每批内存有固定上限。
export async function verifyPublicArtifact({ url, size, fetchImpl = fetch, onProgress = () => {}, blockSize = 8 * 1024 * 1024, concurrency = 4 }) {
  const controller = new AbortController();
  const signal = AbortSignal.any([controller.signal, AbortSignal.timeout(60 * 60 * 1000)]);
  const hash = createHash('sha512'); let transferred = 0; let headers;
  async function block(start, end, ranged) {
    const expected = end - start + 1;
    const response = await portable.checkedFetch(url, fetchImpl, { signal, ...(ranged ? { headers: { Range: `bytes=${start}-${end}` } } : {}) });
    headers ??= response.headers;
    if (ranged && (response.status !== 206 || response.headers.get('content-range') !== `bytes ${start}-${end}/${size}`)) throw new Error('公网更新源 Range 响应不匹配');
    const length = response.headers.get('content-length');
    if (length !== null && Number(length) !== expected) throw new Error('公网回读响应长度不匹配');
    if (response.headers.get('content-encoding') && response.headers.get('content-encoding') !== 'identity') throw new Error('公网回读传输编码不匹配');
    const chunks = []; let received = 0;
    for await (const bytes of response.body) {
      received += bytes.length;
      if (received > expected) throw new Error('公网回读超过声明长度');
      chunks.push(Buffer.from(bytes)); transferred += bytes.length; onProgress(transferred);
    }
    if (received !== expected) throw new Error('公网回读文件不完整');
    return Buffer.concat(chunks);
  }
  try {
    if (size <= blockSize) hash.update(await block(0, size - 1, false));
    else {
      for (let start = 0; start < size; start += blockSize * concurrency) {
        const batch = [];
        for (let slot = 0; slot < concurrency && start + slot * blockSize < size; slot++) {
          const offset = start + slot * blockSize;
          batch.push(block(offset, Math.min(offset + blockSize, size) - 1, true));
        }
        for (const bytes of await Promise.all(batch)) hash.update(bytes);
      }
    }
    return { size: transferred, digest: hash.digest('base64'), headers };
  } finally { controller.abort(); }
}
