const { createHash } = require("node:crypto");
const types = { pdf: "application/pdf", docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document", txt: "text/plain", md: "text/markdown", png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg" };
function safeName(value) {
  if (typeof value !== "string" || !value.trim() || value.length > 240 || /[\\/\u0000-\u001f\u007f]/.test(value) || [".",".."].includes(value.trim())) throw new Error("附件文件名无效");
  return value.trim().normalize("NFC");
}
function responseName(header) {
  if (!header) return null;
  const encoded = /(?:^|;)\s*filename\*\s*=\s*UTF-8''([^;]+)/i.exec(header);
  if (encoded) {
    try { return safeName(decodeURIComponent(encoded[1].trim())); }
    catch { throw new Error("附件响应文件名编码或路径无效"); }
  }
  const plain = /(?:^|;)\s*filename\s*=\s*(?:"([^"]*)"|([^;]*))/i.exec(header);
  return plain ? safeName(plain[1] ?? plain[2]) : null;
}
async function downloadCollectionAttachment(fetch, args, origins, refuse = () => null) {
  const declaredName = safeName(args.name);
  const check = value => {
    const parsed = new URL(value);
    if (!["http:", "https:"].includes(parsed.protocol) || parsed.username || parsed.password || !origins.includes(parsed.origin) || refuse(value)) throw new Error("附件或跳转目标超出本次授权来源");
    return parsed.href;
  };
  let current = check(args.url);
  const signal = AbortSignal.timeout(30000);
  for (let redirects = 0; redirects <= 5; redirects++) {
    const response = await fetch(current, {method:"GET", credentials:"include", redirect:"manual", cache:"no-store", signal});
    if ([301,302,303,307,308].includes(response.status)) {
      await response.body?.cancel();
      const target = response.headers.get("location");
      if (!target || redirects === 5) throw new Error("附件跳转无效或次数超限");
      current = check(new URL(target, current).href);
      continue;
    }
    if (!response.ok) { await response.body?.cancel(); throw new Error(`附件下载失败（${response.status}）`); }
    const contentType = (response.headers.get("content-type") || "").split(";")[0].trim().toLowerCase();
    let name;
    try { name = responseName(response.headers.get("content-disposition")) || declaredName; }
    catch (error) { await response.body?.cancel(); throw error; }
    let ext = name.split(".").at(-1).toLowerCase();
    if (!types[ext]) {
      const responseFormat = Object.keys(types).find(format => types[format] === contentType);
      if (!responseFormat) { await response.body?.cancel(); throw new Error("附件格式不支持或实际格式无法确认：" + name); }
      name = safeName(name + "." + responseFormat);
      ext = responseFormat;
    }
    const expected = types[ext];
    if (contentType !== expected && contentType !== "application/octet-stream" && !(ext === "md" && contentType === "text/plain")) { await response.body?.cancel(); throw new Error("附件实际类型与文件名不一致"); }
    if (Number(response.headers.get("content-length")) > 20 * 1024 * 1024) { await response.body?.cancel(); throw new Error("附件超过20MB"); }
    if (!response.body) throw new Error("附件下载未返回原件");
    const reader = response.body.getReader(), chunks = [];
    let size = 0;
    for (;;) {
      const {done,value} = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > 20 * 1024 * 1024) { await reader.cancel(); throw new Error("附件超过20MB"); }
      chunks.push(Buffer.from(value));
    }
    const bytes = Buffer.concat(chunks);
    if (!bytes.length) throw new Error("附件原件为空");
    const valid = ext === "pdf" ? bytes.subarray(0,5).toString() === "%PDF-" : ext === "docx" ? bytes[0] === 80 && bytes[1] === 75 : ext === "png" ? bytes.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10])) : ["jpg","jpeg"].includes(ext) ? bytes[0] === 255 && bytes[1] === 216 && bytes[2] === 255 : true;
    if (!valid) throw new Error("附件原件签名与实际格式不一致");
    if (["txt","md"].includes(ext)) {
      try { new TextDecoder("utf-8", {fatal:true}).decode(bytes); }
      catch { throw new Error("文本附件须使用UTF-8编码"); }
    }
    return {url:args.url, finalUrl:current, name, contentType:expected, sha256:createHash("sha256").update(bytes).digest("hex"), obtainedAt:new Date().toISOString(), base64:bytes.toString("base64")};
  }
  throw new Error("附件跳转次数超限");
}
module.exports = { downloadCollectionAttachment };
