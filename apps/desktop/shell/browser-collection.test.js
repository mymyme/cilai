const test = require("node:test");
const assert = require("node:assert/strict");
const { JSDOM } = require("jsdom");
const { collectDocument } = require("./browser-collection");
function collect(html, selector = "article", links = "", attachmentCapture = true) {
  const dom = new JSDOM(html, {url:"https://example.com/policy/index.html",runScripts:"outside-only"});
  try { return dom.window.eval(`(${collectDocument.toString()})(${JSON.stringify(selector)},${JSON.stringify(links)},${JSON.stringify(attachmentCapture)})`); }
  finally { dom.window.close(); }
}
test("采集保留正文表格、绝对链接及已有元数据，排除导航和脚本", () => {
  const page = collect('<meta name="ArticleTitle" content="通知"><meta name="PubDate" content="2026-10-16 12:00"><nav>网站导航</nav><article><h1>通知</h1><p>正文 <a href="../file.pdf">附件</a></p><table><tr><th>部门</th><th>频率</th></tr><tr><td>环保部</td><td>每周</td></tr></table><script>secret()</script><p hidden>隐藏</p></article>');
  assert.equal(page.title,"通知");assert.equal(page.published,"2026-10-16");
  assert.ok(page.text.includes('[附件](<https://example.com/file.pdf>)'));
  assert.ok(page.text.includes('| 部门 | 频率 |\n| --- | --- |\n| 环保部 | 每周 |'));
  assert.ok(!page.text.includes('secret') && !page.text.includes('网站导航') && !page.text.includes('隐藏'));
});
test("重叠正文容器不重复采集，正文缺失和过长明确失败", () => {
  assert.equal(collect('<article><div id="body">唯一正文</div></article>', 'article,#body').text,"唯一正文");
  assert.throws(()=>collect('<main>没有正文</main>'), /未找到正文元素/);
  assert.throws(()=>collect('<article>'+ '字'.repeat(200001)+'</article>'), /超过采集限额/);
});
test("不生成执行型链接，缺少日期保持空值", () => {
  const page=collect('<article><a href="javascript:alert(1)">文本</a></article>');
  assert.equal(page.text,"文本");assert.equal(page.published,"");
});

test("附件独立声明并从后续HTML链接排除，不截断或将旧执行端当作已下载", () => {
  const page = collect('<article><p>通知</p><a href="/a.pdf">附件一</a><a href="/a.pdf">重复</a><a href="/next">下一页</a><a href="/old.doc">旧格式</a></article>', 'article', 'a');
  assert.equal(page.attachments.length,2);
  assert.equal(page.attachments[0].name,'a.pdf');
  const dynamic = collect('<article>正文<a href="/download?id=1">中文附件.docx</a></article>');
  assert.equal(dynamic.attachments[0].name,'中文附件.docx');
  const unknown = collect('<article>正文<a href="/download?id=123">附件下载</a></article>');
  assert.equal(unknown.attachments.length,1);
  assert.throws(()=>collect('<article>正文<a href="/download?id=123">附件下载</a></article>','article','',false),/升级工作台/);
  assert.deepEqual(Array.from(page.links),['https://example.com/next']);
  assert.throws(()=>collect('<article>正文<a href="/a.pdf">附件</a></article>','article','',false),/升级工作台/);
  assert.equal(collect('<article>正文</article>','article','',false).text,'正文');
  assert.throws(()=>collect('<article>正文'+Array.from({length:9},(_,n)=>`<a href="/${n}.pdf">附件</a>`).join('')+'</article>'),/超过八项/);
});
