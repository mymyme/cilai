const test = require("node:test");
const assert = require("node:assert/strict");
const { createPersonalDocuments, ORIGIN, MAX_DOCUMENT_BYTES, MAX_DOCUMENTS } = require("./personal-documents");
function fixture() {
  const owner = { isDestroyed: () => false, mainFrame: { url: "app://localhost/" } };
  return { owner, event: { sender: owner, senderFrame: owner.mainFrame }, documents: createPersonalDocuments(() => owner) };
}
const create = (documents, event, html = "<h1>测试</h1>") => documents.invoke(event, { command: "personal_document_create", args: { html } });

test("只有工作台主框架可登记页面", () => {
  const { owner, event, documents } = fixture();
  assert.throws(() => create(documents, { ...event, sender: {} }), /来源无效/);
  assert.throws(() => create(documents, { ...event, senderFrame: { url: "app://localhost/" } }), /来源无效/);
  owner.mainFrame.url = "app://personal-capability/other";
  assert.throws(() => create(documents, event), /来源无效/);
  owner.mainFrame.url = "https://localhost/";
  assert.throws(() => create(documents, event), /来源无效/);
});

test("页面使用独立来源、响应级隔离和无缓存策略", async () => {
  const { event, documents } = fixture();
  const url = create(documents, event);
  assert.ok(url.startsWith(`${ORIGIN}/`));
  const response = documents.response({ url, method: "GET" });
  assert.equal(await response.text(), "<h1>测试</h1>");
  assert.equal(response.headers.get("Cache-Control"), "no-store");
  const policy = response.headers.get("Content-Security-Policy");
  for (const rule of ["script-src 'unsafe-inline'", "connect-src 'none'", "sandbox allow-scripts", "frame-ancestors app://localhost"]) assert.ok(policy.includes(rule));
  assert.ok(!policy.includes("allow-same-origin"));
  for (const request of [{ url: `${url}?x=1`, method: "GET" }, { url, method: "POST" }, { url: `${ORIGIN}/missing`, method: "GET" }]) {
    assert.equal(documents.response(request).status, 404);
  }
  documents.invoke(event, { command: "personal_document_release", args: { url } });
  assert.equal(documents.response({ url, method: "GET" }).status, 404);
});

test("页面内存有界，释放和窗口清理回收容量", () => {
  const { event, documents } = fixture();
  assert.throws(() => create(documents, event, "a".repeat(MAX_DOCUMENT_BYTES + 1)), /加载限制/);
  const urls = Array.from({ length: MAX_DOCUMENTS }, () => create(documents, event));
  assert.throws(() => create(documents, event), /加载限制/);
  documents.invoke(event, { command: "personal_document_release", args: { url: urls[0] } });
  create(documents, event);
  documents.clear();
  for (const url of urls) assert.equal(documents.response({ url, method: "GET" }).status, 404);
  for (let i = 0; i < 8; i++) create(documents, event, "a".repeat(MAX_DOCUMENT_BYTES));
  assert.throws(() => create(documents, event), /加载限制/);
  documents.clear();
  create(documents, event);
});
