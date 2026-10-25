const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { createHash } = require("node:crypto");
const { createDiagnosticExport } = require("./diagnostic-export");
const { createTestDiagnostics } = require("./test-diagnostics");

function setup(t) {
  const fixtures = path.join(require("./personal-isolation").personalBuildRoot(), "verification/diagnostic-fixtures");
  fs.mkdirSync(fixtures,{recursive:true});
  const directory = fs.mkdtempSync(path.join(fixtures, "diagnostic-"));
  t.after(() => fs.rmSync(directory,{recursive:true,force:true}));
  const diagnostics = createTestDiagnostics({directory:path.join(directory,"logs"),metadata:()=>({version:"test",platform:process.platform})});
  let clock=Date.now(), collects=0, dialogs=0;
  const selected = path.join(directory,"saved.txt");
  const exporter = createDiagnosticExport({diagnostics:()=>diagnostics,collect:async()=>{collects++; return {status:"collected",profileStatus:"active"};},
    now:()=>clock, dialog:{showSaveDialog:async()=>{dialogs++; return {filePath:selected,canceled:false};}}});
  return {directory,diagnostics,exporter,selected,advance:()=>{clock+=600001;},counts:()=>({collects,dialogs})};
}
test("preview exports exactly the frozen sanitized snapshot after later records arrive", async t => {
  const s=setup(t);
  s.diagnostics.response("model_connection_validate",null,"model_validation_network_failed api_key=PRIVATE_SECRET https://PRIVATE_HOST/prompt",1);
  const preview=await s.exporter.preview(1);
  assert.equal(fs.existsSync(s.selected),false);
  assert.match(preview.text,/分类 网络/);
  assert.equal(preview.text.includes("PRIVATE"),false);
  s.diagnostics.response("model_connection_validate",null,"model_validation_authentication_failed",2);
  assert.equal(await s.exporter.save(1,null,{previewId:preview.previewId,digest:preview.digest}),true);
  const text=fs.readFileSync(s.selected,"utf8");
  assert.equal(text,preview.text);
  assert.equal(createHash("sha256").update(text).digest("hex"),preview.digest);
  assert.equal(text.includes("model_validation_authentication_failed"),false);
  assert.deepEqual(s.counts(),{collects:1,dialogs:1});
});
test("wrong window, changed receipt and expired preview cannot reach native file selection", async t => {
  const s=setup(t), preview=await s.exporter.preview(1), args={previewId:preview.previewId,digest:preview.digest};
  await assert.rejects(s.exporter.save(2,null,args),/预览已失效/);
  await assert.rejects(s.exporter.save(1,null,{...args,digest:"b".repeat(64)}),/预览已失效/);
  await assert.rejects(s.exporter.save(1,null,{...args,filePath:s.selected}),/预览已失效/);
  s.advance();
  await assert.rejects(s.exporter.save(1,null,args),/预览已失效/);
  assert.deepEqual(s.counts(),{collects:1,dialogs:0});
});
test("replacing a preview invalidates earlier confirmation without collecting during save", async t => {
  const s=setup(t), previous=await s.exporter.preview(1), current=await s.exporter.preview(1);
  await assert.rejects(s.exporter.save(1,null,{previewId:previous.previewId,digest:previous.digest}),/预览已失效/);
  await s.exporter.save(1,null,{previewId:current.previewId,digest:current.digest});
  assert.deepEqual(s.counts(),{collects:2,dialogs:1});
});
test("failed save preserves original target and no success receipt is returned", async t => {
  const s=setup(t), preview=await s.exporter.preview(1);
  fs.mkdirSync(s.selected);
  fs.writeFileSync(path.join(s.selected,"user-data"),"kept");
  await assert.rejects(s.exporter.save(1,null,{previewId:preview.previewId,digest:preview.digest}),/未能保存/);
  assert.equal(fs.readFileSync(path.join(s.selected,"user-data"),"utf8"),"kept");
  assert.equal(fs.readdirSync(s.directory).some(name=>name.endsWith(".tmp")),false);
});
