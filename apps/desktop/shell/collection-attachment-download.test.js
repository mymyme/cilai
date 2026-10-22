const test = require("node:test");
const assert = require("node:assert/strict");
const {createHash} = require("node:crypto");
const {downloadCollectionAttachment:download} = require("./collection-attachment-download");
const args={url:"https://policy.test/a.txt",name:"附件.txt"}, origins=["https://policy.test"];
test("原件按当前会话下载且字节摘要一致，逐次校验重定向目标", async()=>{
  const calls=[],bytes=Buffer.from("附件原文");
  const evidence=await download(async(url,init)=>{calls.push({url,init});return calls.length===1?new Response(null,{status:302,headers:{location:"/real.txt"}}):new Response(bytes,{headers:{"content-type":"text/plain"}});},args,origins);
  assert.equal(calls.length,2);assert.equal(calls[0].init.credentials,"include");assert.equal(calls[0].init.redirect,"manual");
  assert.equal(evidence.finalUrl,"https://policy.test/real.txt");assert.equal(evidence.base64,bytes.toString("base64"));assert.equal(evidence.sha256,createHash("sha256").update(bytes).digest("hex"));
  let count=0;await assert.rejects(download(async()=>{count++;return new Response(null,{status:302,headers:{location:"https://outside.test/x"}});},args,origins),/超出/);assert.equal(count,1);
});
test("不支持格式、登录页、失败响应和流式超限不形成附件成功证据", async()=>{
  await assert.rejects(download(async()=>new Response("ZIP",{headers:{"content-type":"application/zip"}}),{...args,name:"附件.zip"},origins),/不支持/);
  await assert.rejects(download(async()=>new Response("登录",{headers:{"content-type":"text/html"}}),args,origins),/类型/);
  await assert.rejects(download(async()=>new Response("拒绝",{status:403}),args,origins),/403/);
  const body=new ReadableStream({start(controller){controller.enqueue(Buffer.alloc(20*1024*1024));controller.enqueue(Buffer.alloc(1));controller.close();}});
  await assert.rejects(download(async()=>new Response(body,{headers:{"content-type":"text/plain"}}),args,origins),/20MB/);
});

test("动态附件通过真实HTTP响应文件名、类型和字节签名确认原件", async()=>{
  const http=require("node:http"), bytes=Buffer.from("%PDF-1.7\n真实PDF字节");
  const server=http.createServer((req,res)=>{
    if(req.url==="/download?id=123") { res.writeHead(200,{"content-type":"application/pdf","content-disposition":"attachment; filename*=UTF-8''"+encodeURIComponent("中文附件.pdf")});res.end(bytes); }
    else if(req.url==="/mime-only") {res.writeHead(200,{"content-type":"application/pdf"});res.end(bytes);}
    else if(req.url==="/unsafe") {res.writeHead(200,{"content-type":"application/pdf","content-disposition":'attachment; filename="../bad.pdf"'});res.end(bytes);}
    else {res.writeHead(302,{location:"http://outside.test/file.pdf"});res.end();}
  });
  await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));
  const origin="http://127.0.0.1:"+server.address().port;
  try {
    const evidence=await download(fetch,{url:origin+"/download?id=123",name:"附件下载"},[origin]);
    assert.equal(evidence.name,"中文附件.pdf");assert.equal(evidence.contentType,"application/pdf");assert.equal(evidence.base64,bytes.toString("base64"));
    assert.equal((await download(fetch,{url:origin+"/mime-only",name:"附件下载"},[origin])).name,"附件下载.pdf");
    await assert.rejects(download(fetch,{url:origin+"/unsafe",name:"附件下载"},[origin]),/文件名/);
    await assert.rejects(download(fetch,{url:origin+"/redirect",name:"附件下载"},[origin]),/超出/);
  } finally { await new Promise(resolve=>server.close(resolve)); }
});
