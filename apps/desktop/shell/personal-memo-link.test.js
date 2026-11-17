const test = require('node:test');
const assert = require('node:assert/strict');
const { openSavedMemoLink } = require('./personal-memo-link');
test('只读取当前资料中已保存的链接，不信任界面传入地址',async()=>{
  const calls=[],opened=[];
  await openSavedMemoLink({service:{async call(...args){calls.push(args);return {url:'https://example.com/saved'};}},args:{runtimeSessionId:'session',id:'memo',url:'https://evil.example'},openExternal:async url=>opened.push(url)});
  assert.deepEqual(calls,[['ai_pa_records',{runtimeSessionId:'session',kind:'memo',request:{action:'get',id:'memo'}}]]);assert.deepEqual(opened,['https://example.com/saved']);
});
test('拒绝执行型链接、文件路径和带凭据的地址',async()=>{
  for(const url of ['javascript:alert(1)','file:///tmp/private','https://name:secret@example.com/'])await assert.rejects(openSavedMemoLink({service:{async call(){return {url};}},args:{},openExternal(){throw new Error('must not open');}}),/pa_memo_url_invalid/);
});
