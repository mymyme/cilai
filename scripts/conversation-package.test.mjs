import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { verifyConversationPackage } from "./conversation-package.mjs";
const hash=value=>createHash("sha256").update(value).digest("hex");
function fixture(context){
 const base=mkdtempSync(join(tmpdir(),"conversation-package-"));context.after(()=>rmSync(base,{recursive:true,force:true}));
 const root=join(base,"词来工作台/06代码索引/cilai-workbench"),shared=join(root,"packages/app-ui");
 mkdirSync(join(root,"packages"),{recursive:true});mkdirSync(join(shared,"src/conversation"),{recursive:true});
 const pkg='{"name":"@cilai/app-ui","version":"1.1.0"}\n',src='export const shared = true;\n',license='Apache license\n',notice='Shared component notice\n';
 writeFileSync(join(shared,"package.json"),pkg);writeFileSync(join(shared,"src/conversation/index.tsx"),src);
 writeFileSync(join(shared,"LICENSE"),license);writeFileSync(join(shared,"NOTICE"),notice);
 writeFileSync(join(root,"packages/cilai-app-ui-1.1.0.tgz"),"package-bytes");
 writeFileSync(join(root,"packages/conversation-package.json"),JSON.stringify({name:"@cilai/app-ui",version:"1.1.0",archive:"cilai-app-ui-1.1.0.tgz",sha256:hash("package-bytes"),sources:{"package.json":hash(pkg),"LICENSE":hash(license),"NOTICE":hash(notice),"src/conversation/index.tsx":hash(src)}}));
 return {root,shared};
}
test("打包组件内容被替换时停止构建",context=>{
 const {root}=fixture(context);assert.doesNotThrow(()=>verifyConversationPackage(root));
 writeFileSync(join(root,"packages/cilai-app-ui-1.1.0.tgz"),"changed-package");assert.throws(()=>verifyConversationPackage(root),/包校验失败/);
});
test("共享源码更新而未重新生成组件包时停止构建",context=>{
 const {root,shared}=fixture(context);writeFileSync(join(shared,"src/conversation/index.tsx"),"export const shared = false;");
 assert.throws(()=>verifyConversationPackage(root),/共享组件源码已改变/);
});

test("Windows CRLF 检出与同一份 LF 源码匹配，内容改动仍被拒绝",context=>{
 const {root,shared}=fixture(context);
 for(const name of ["package.json","LICENSE","NOTICE","src/conversation/index.tsx"]){
  const path=join(shared,name);writeFileSync(path,readFileSync(path,"utf8").replace(/\n/g,"\r\n"));
 }
 assert.doesNotThrow(()=>verifyConversationPackage(root));
 writeFileSync(join(shared,"src/conversation/index.tsx"),"export const shared = false;\r\n");
 assert.throws(()=>verifyConversationPackage(root),/共享组件源码已改变/);
});

for (const name of ['LICENSE', 'NOTICE']) test(`共享组件 ${name} 改动未同步归档时停止构建`, context => {
 const {root,shared}=fixture(context);writeFileSync(join(shared,name),'Changed notice');
 assert.throws(()=>verifyConversationPackage(root),/共享组件源码已改变/);
});

test("桌面对话包不受未使用的知识库页面源码影响",context=>{
 const {root,shared}=fixture(context);writeFileSync(join(shared,"src/KnowledgeWorkspace.tsx"),"export const unrelated = true;");
 assert.doesNotThrow(()=>verifyConversationPackage(root));
});
