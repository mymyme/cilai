const test = require('node:test');
const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const { installPersonalReminders } = require('./personal-reminders');
const flush = () => new Promise(resolve => setImmediate(resolve));
function fixture(t, enabled = true) {
  t.mock.timers.enable({apis:['setInterval']});
  const powerMonitor = new EventEmitter(), calls = [], shown = [], events = [];
  let notices = [], deferred;
  class Notification extends EventEmitter {
    static isSupported() { return true; }
    constructor(options) { super(); this.options = options; }
    show() { shown.push(this); }
    close() { this.emit('close'); }
  }
  const service = {child:{},async call(command,args) {
    calls.push([command,args]);
    if (command === 'ai_pa_reminder_tick') { if(deferred)return deferred; const desktop=notices; notices=[]; return {runtimeSessionId:'session',desktop,changed:desktop.length>0}; }
    if (command === 'ai_pa_reminder_claim') return true;
    if (command === 'ai_pa_reminder_result') return null;
    throw new Error(`unexpected command ${command}`);
  }};
  const reminders = installPersonalReminders({service,powerMonitor,Notification,publish:(...args)=>events.push(args),showWindow(){events.push(['show']);},log:err=>{throw err;},notificationSettings:async()=>({enabled,sound:false})});
  t.after(()=>reminders.stop());
  return {calls,shown,events,powerMonitor,reminders,due:()=>{notices=[{id:'notice',todoId:'todo'}];},defer:promise=>{deferred=promise;}};
}
test('待办先领取再提交系统通知，保持静音设置，点击只导航且不调用模型',async t=>{
  const f=fixture(t);await flush();f.due();t.mock.timers.tick(5000);await flush();
  assert.equal(f.shown.length,1);assert.equal(f.shown[0].options.silent,true);
  assert.deepEqual(f.calls.map(([name])=>name),['ai_pa_reminder_tick','ai_pa_reminder_tick','ai_pa_reminder_claim','ai_pa_reminder_result']);
  f.shown[0].emit('click');assert.deepEqual(f.events.at(-1),['personal-reminder-open',{runtimeSessionId:'session',todoId:'todo'}]);
  t.mock.timers.tick(5000);await flush();assert.equal(f.shown.length,1);
});
test('关闭系统通知仍记录提醒结果，不显示通知也不发起模型执行',async t=>{
  const f=fixture(t,false);await flush();f.due();t.mock.timers.tick(5000);await flush();
  assert.equal(f.shown.length,0);assert.equal(f.calls.at(-1)[0],'ai_pa_reminder_result');assert.equal(f.calls.at(-1)[1].outcome,'disabled');
});
test('休眠停止轮询，恢复明确核对错过状态，资料切换使迟到通知和旧点击失效',async t=>{
  const f=fixture(t);await flush();const before=f.calls.length;f.powerMonitor.emit('suspend');t.mock.timers.tick(5000);await flush();assert.equal(f.calls.length,before);
  f.powerMonitor.emit('resume');await flush();assert.equal(f.calls.at(-1)[1].waking,true);
  let release;f.defer(new Promise(resolve=>{release=resolve;}));t.mock.timers.tick(5000);await flush();
  const complete=f.reminders.beginControl('personal_profile_close');release({runtimeSessionId:'session',desktop:[{id:'late',todoId:'todo'}]});await flush();assert.equal(f.shown.length,0);f.defer(null);complete();await flush();
});
