const { test } = require('node:test');
const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const { createPersonalNotifications, PRIVATE_COMMANDS } = require('./personal-notifications');

const PROFILE = 'a'.repeat(32);
const OTHER = 'b'.repeat(32);
function candidate(index, kind = 'completed', change = {}) {
  return { eventId: require('node:crypto').createHash('sha256').update(String(index)).digest('hex'), kind, occurredAt: 100,
    target: { kind: 'run', runId: `run-${index}`, conversationId: 'conversation' },
    permissionId: null, confirmationId: null, validUntil: null, enabledForDelivery: true, ...change };
}

function fixture(options = {}) {
  const calls = [], natives = [], timeline = [], navigations = [];
  const records = options.records ?? new Map();
  const snapshot = { profileId: PROFILE, sourceEpoch: 'epoch', observedAt: 100,
    preferences: { enabled: options.enabled ?? true, completed: true, failed: true, actionRequired: true, sound: false, revision: 0 }, events: [] };
  let mode = options.mode ?? 'shown';
  let intercept = null;
  class Native extends EventEmitter {
    static isSupported() { return options.supported ?? true; }
    static async getHistory() { calls.push(['history']); return options.history ?? []; }
    static handleActivation(callback) { Native.activation = callback; }
    constructor(properties) {
      super(); this.id = properties.id; this.properties = properties; this.showCount = 0; this.closeCount = 0;
      natives.push(this); timeline.push('construct');
    }
    show() {
      this.showCount++; timeline.push('show');
      if (mode === 'shown') queueMicrotask(() => this.emit('show'));
      if (mode === 'failed') queueMicrotask(() => this.emit('failed', {}, 'secret-from-native-must-not-be-stored'));
      if (mode === 'closed') queueMicrotask(() => this.emit('close'));
    }
    close() { this.closeCount++; }
  }
  const service = { async call(command, args) {
    calls.push([command, structuredClone(args)]);
    if (intercept) {
      const result = intercept(command, args);
      if (result !== undefined) return result;
    }
    if (command === PRIVATE_COMMANDS.read) return structuredClone(snapshot);
    if (command === PRIVATE_COMMANDS.claim) {
      timeline.push('claim');
      const event = snapshot.events.find(item => item.eventId === args.request.eventId);
      assert.ok(event);
      const known = records.get(event.eventId);
      if (known) return { mayDeliver: false, delivery: structuredClone(known), sound: false };
      const delivery = { notificationId: event.eventId.slice(0, 32), profileId: args.request.profileId,
        sourceEpoch: args.request.sourceEpoch, event: structuredClone(event), outcome: 'claimed', claimedAt: 100, reportedAt: null, reasonCode: null };
      records.set(event.eventId, delivery);
      return { mayDeliver: true, delivery: structuredClone(delivery), sound: false };
    }
    if (command === PRIVATE_COMMANDS.report) {
      const delivery = [...records.values()].find(item => item.notificationId === args.request.notificationId);
      assert.ok(delivery); Object.assign(delivery, args.request); return structuredClone(delivery);
    }
    if (command === PRIVATE_COMMANDS.open) {
      assert.equal(args.request.profileId, snapshot.profileId);
      return [...records.values()].find(item => item.notificationId === args.request.notificationId)?.event.target;
    }
    if (command === 'personal_notification_preferences_get') return { profileId: snapshot.profileId, preferences: snapshot.preferences };
    if (command === 'personal_notification_preferences_save') {
      Object.assign(snapshot.preferences, args.request);
      return { profileId: snapshot.profileId, preferences: snapshot.preferences };
    }
    throw new Error(`unexpected ${command}`);
  } };
  const windowCalls = [];
  const notifier = createPersonalNotifications({ Notification: Native, service,
    platform: options.platform ?? 'darwin', headless: options.headless ?? false,
    getWindow: () => ({ isDestroyed: () => false, isMinimized: () => true,
      restore: () => windowCalls.push('restore'), show: () => windowCalls.push('show'), focus: () => windowCalls.push('focus') }),
    navigate: async value => { navigations.push(value); }, now: () => 100,
    outcomeTimeoutMs: 25, requestTimeoutMs: 100,
  });
  notifier.setProfile(PROFILE);
  return { notifier, Native, snapshot, calls, records, natives, timeline, navigations, windowCalls,
    intercept(value) { intercept = value; }, mode(value) { mode = value; } };
}

async function start(f) { f.notifier.start(); await f.notifier.refresh(); }
const commands = f => f.calls.filter(([command]) => command !== 'history').map(([command]) => command);
const settle = () => new Promise(resolve => setImmediate(resolve));

test('baseline is read-only and a new transition claims durably before native construction', async t => {
  const f = fixture(); t.after(() => f.notifier.stop()); f.snapshot.events.push(candidate(1)); await start(f);
  assert.equal(f.natives.length, 0);
  f.snapshot.events.push(candidate(2)); await f.notifier.refresh(); await f.notifier.refresh();
  assert.deepEqual(f.timeline, ['claim', 'construct', 'show']);
  assert.equal(f.records.get(candidate(2).eventId).outcome, 'shown');
  assert.equal(commands(f).filter(command => command === PRIVATE_COMMANDS.claim).length, 1);
});

test('disabled preferences do not touch OS history or construct notices and enabling does not replay the baseline', async t => {
  const f = fixture({ enabled: false }); t.after(() => f.notifier.stop()); await start(f);
  f.snapshot.events.push(candidate(3)); await f.notifier.refresh();
  assert.equal(f.calls.filter(([command]) => command === 'history').length, 0);
  f.snapshot.preferences.enabled = true; await f.notifier.refresh(); assert.equal(f.natives.length, 0);
  f.snapshot.events.push(candidate(4)); await f.notifier.refresh(); assert.equal(f.natives.length, 1);
  f.snapshot.preferences.enabled = false; await f.notifier.refresh(); assert.equal(f.natives[0].closeCount, 1);
});

test('unknown native result remains unknown and a later appearance cannot obtain a second delivery permit', async t => {
  const f = fixture({ mode: 'unknown' }); await start(f); f.snapshot.events.push(candidate(5)); await f.notifier.refresh();
  assert.equal(f.records.get(candidate(5).eventId).outcome, 'unknown'); await f.notifier.stop();
  const restarted = fixture({ records: f.records }); t.after(() => restarted.notifier.stop()); await start(restarted);
  restarted.snapshot.events.push(candidate(5)); await restarted.notifier.refresh();
  assert.equal(restarted.natives.length, 0); assert.equal(restarted.records.size, 1);
});

test('a claim transport error is never converted to native success or automatically resubmitted', async t => {
  const f = fixture(); t.after(() => f.notifier.stop()); await start(f);
  f.intercept(command => { if (command === PRIVATE_COMMANDS.claim) throw new Error('lost response'); });
  f.snapshot.events.push(candidate(6)); await f.notifier.refresh(); await f.notifier.refresh();
  assert.equal(f.natives.length, 0); assert.equal(commands(f).filter(command => command === PRIVATE_COMMANDS.claim).length, 1);
});

test('expired confirmations and disabled automation completion candidates cannot deliver', async t => {
  const f = fixture(); t.after(() => f.notifier.stop()); await start(f);
  f.snapshot.events.push(candidate(7, 'action_required', { validUntil: 99 }), candidate(8, 'completed', { enabledForDelivery: false }));
  await f.notifier.refresh(); assert.equal(f.natives.length, 0);
  f.snapshot.events.push(candidate(9, 'action_required', { validUntil: 500 })); await f.notifier.refresh();
  assert.equal(f.natives[0].properties.body, '任务需要确认');
});

test('native failure is explicit and neither native diagnostics nor event text can enter OS messages or receipts', async t => {
  const f = fixture({ mode: 'failed' }); t.after(() => f.notifier.stop()); await start(f);
  f.snapshot.events.push(candidate(10, 'failed', { title: 'private title', body: 'private prompt', command: 'private command' }));
  await f.notifier.refresh();
  assert.equal(f.natives[0].properties.body, '任务未完成'); assert.equal(f.natives[0].properties.title, '词来个人工作台');
  assert.equal(f.natives[0].properties.silent, true); assert.equal(f.natives[0].properties.hasReply, false);
  const report = f.calls.find(([command]) => command === PRIVATE_COMMANDS.report)[1];
  assert.equal(report.request.outcome, 'failed'); assert.equal(report.request.reasonCode, 'notification_native_failed');
  assert.doesNotMatch(JSON.stringify(report), /secret-from-native|private (prompt|command|title)/);
});

test('close is not evidence that the system displayed a notification', async t => {
  const f = fixture({ mode: 'closed' }); t.after(() => f.notifier.stop()); await start(f);
  f.snapshot.events.push(candidate(11)); await f.notifier.refresh();
  assert.equal(f.records.get(candidate(11).eventId).outcome, 'unknown');
});

test('profile change while claiming retires the old delivery without invoking OS or navigating another profile', async t => {
  const f = fixture(); t.after(() => f.notifier.stop()); await start(f);
  let continueClaim;
  f.intercept(command => { if (command === PRIVATE_COMMANDS.claim) return new Promise(resolve => { continueClaim = resolve; }); });
  const item = candidate(12); f.snapshot.events.push(item); const pending = f.notifier.refresh(); await settle();
  f.notifier.setProfile(OTHER);
  continueClaim({ mayDeliver: true, sound: false, delivery: { notificationId: item.eventId.slice(0, 32), profileId: PROFILE, event: item } });
  await pending; assert.equal(f.natives.length, 0); assert.equal(f.navigations.length, 0);
});

test('notification click restores the workbench and uses only fresh target resolution without approval or task execution', async t => {
  const f = fixture(); t.after(() => f.notifier.stop()); await start(f); f.snapshot.events.push(candidate(13)); await f.notifier.refresh();
  f.natives[0].emit('click'); await settle();
  assert.deepEqual(f.windowCalls, ['restore', 'show', 'focus']); assert.equal(f.navigations.length, 1);
  assert.deepEqual(f.navigations[0].target, candidate(13).target);
  assert.ok(commands(f).every(command => Object.values(PRIVATE_COMMANDS).includes(command)));
  assert.equal(Object.hasOwn(f.notifier, 'open'), false);
});

test('restoring Mac history attaches click handlers without reposting old native messages', async t => {
  const old = new EventEmitter(); old.id = 'c'.repeat(32); old.show = () => assert.fail('history may not be posted'); old.close = () => {};
  const f = fixture({ history: [old] }); t.after(() => f.notifier.stop());
  const item = candidate(14); f.records.set(item.eventId, { notificationId: old.id, profileId: PROFILE, event: item });
  await start(f); old.emit('click'); await settle(); assert.equal(f.navigations.length, 1); assert.equal(f.natives.length, 0);
});

test('Windows activation accepts only the fixed opaque identifier format', async t => {
  const f = fixture({ platform: 'win32' }); t.after(() => f.notifier.stop()); await start(f);
  f.snapshot.events.push(candidate(15)); await f.notifier.refresh();
  assert.match(f.natives[0].properties.toastXml, /launch="cilai-personal-notification:[a-f0-9]{32}"/);
  f.Native.activation({ type: 'click', arguments: 'https://foreign.invalid/private' }); await settle(); assert.equal(f.navigations.length, 0);
  f.Native.activation({ type: 'click', arguments: `cilai-personal-notification:${f.natives[0].id}` }); await settle(); assert.equal(f.navigations.length, 1);
});

test('headless and unsupported systems never claim an OS delivery or report success', async t => {
  for (const options of [{ headless: true }, { supported: false }]) {
    const f = fixture(options); t.after(() => f.notifier.stop()); await start(f); f.snapshot.events.push(candidate(16)); await f.notifier.refresh();
    assert.equal(f.natives.length, 0); assert.equal(commands(f).includes(PRIVATE_COMMANDS.claim), false);
    assert.equal(f.notifier.status().supported, false); assert.equal(f.notifier.status().lastOutcome, null);
  }
});

test('pause drains the bounded in-flight notification and stops polling until explicit resume', async t => {
  const f = fixture({ mode: 'unknown' }); t.after(() => f.notifier.stop()); await start(f);
  f.snapshot.events.push(candidate(17)); const pending = f.notifier.refresh(); await settle();
  await f.notifier.pause(); await pending;
  const previous = commands(f).length; await f.notifier.refresh(); assert.equal(commands(f).length, previous);
  assert.equal(f.notifier.status().active, false); assert.equal(f.records.get(candidate(17).eventId).outcome, 'unknown');
});

test('cold activation waits for a real profile and consumes its opaque identifier once without showing', async t => {
  const original = fixture(); await start(original);
  original.snapshot.events.push(candidate(90)); await original.notifier.refresh(); await original.notifier.stop();
  const identifier = candidate(90).eventId.slice(0, 32);
  const f = fixture({records: original.records}); t.after(() => f.notifier.stop());
  f.notifier.setProfile(null); f.notifier.activation({identifier}); await start(f);
  assert.equal(commands(f).filter(command => command === PRIVATE_COMMANDS.open).length, 0);
  f.notifier.setProfile(PROFILE); await settle(); await settle();
  assert.equal(commands(f).filter(command => command === PRIVATE_COMMANDS.open).length, 1);
  assert.equal(f.navigations.length, 1); assert.equal(f.natives.length, 0);
  f.notifier.setProfile(PROFILE); await f.notifier.refresh(); await settle();
  assert.equal(commands(f).filter(command => command === PRIVATE_COMMANDS.open).length, 1);
});

test('explicit maintenance rejection preserves a candidate until resume, without retrying an unknown claim', async t => {
  const f = fixture(); t.after(() => f.notifier.stop()); await start(f);
  let blocked = true;
  f.intercept(command => { if (command === PRIVATE_COMMANDS.claim && blocked) return Promise.reject(new Error('personal_data_maintenance_active')); });
  f.snapshot.events.push(candidate(91)); await f.notifier.refresh();
  assert.equal(f.natives.length, 0);
  await f.notifier.pause(); blocked = false; f.notifier.resume(); await f.notifier.refresh();
  assert.equal(f.natives.length, 1); assert.equal(f.records.get(candidate(91).eventId).outcome, 'shown');
  await f.notifier.refresh(); assert.equal(f.natives[0].showCount, 1);
});

test('pause settles a pending native outcome as unknown and resume never resends it', async t => {
  const f = fixture({mode:'unknown'}); t.after(() => f.notifier.stop()); await start(f);
  f.snapshot.events.push(candidate(92)); const delivery = f.notifier.refresh();
  await settle(); const paused = f.notifier.pause(); await delivery; await paused;
  assert.equal(f.records.get(candidate(92).eventId).outcome, 'unknown');
  f.notifier.resume(); await f.notifier.refresh();
  assert.equal(commands(f).filter(command => command === PRIVATE_COMMANDS.claim).length, 1);
  assert.equal(f.natives.length, 1); assert.equal(f.natives[0].showCount, 1);
});

test('paused settings fail explicitly before contacting the host', async t => {
  const f = fixture(); t.after(() => f.notifier.stop()); await start(f); await f.notifier.pause();
  const before = f.calls.length;
  await assert.rejects(f.notifier.preferences('personal_notification_preferences_get'), /personal_notification_paused/);
  assert.equal(f.calls.length, before); assert.equal(f.notifier.status().reasonCode, 'personal_notification_paused');
});

test('successful global disable takes effect before a pending claim can create a native notification', async t => {
  const f = fixture(); t.after(() => f.notifier.stop()); await start(f);
  let release;
  f.intercept((command,args) => {
    if (command !== PRIVATE_COMMANDS.claim) return undefined;
    const event=f.snapshot.events.find(item=>item.eventId===args.request.eventId);
    const receipt={notificationId:event.eventId.slice(0,32),profileId:PROFILE,sourceEpoch:'epoch',event,
      outcome:'claimed',claimedAt:100,reportedAt:null,reasonCode:null};
    f.records.set(event.eventId,receipt);
    return new Promise(resolve=>{release=()=>resolve({mayDeliver:true,delivery:receipt,sound:false});});
  });
  f.snapshot.events.push(candidate(93));const delivery=f.notifier.refresh();await settle();
  const saving=f.notifier.preferences('personal_notification_preferences_save',{request:{enabled:false}});
  await settle();assert.equal(f.snapshot.preferences.enabled,false);
  release();await delivery;await saving;
  assert.equal(f.natives.length,0);assert.equal(f.records.get(candidate(93).eventId).outcome,'unknown');
  await f.notifier.refresh();assert.equal(f.natives.length,0);
});
