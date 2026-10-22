const { createHash } = require('node:crypto');

const PRIVATE_COMMANDS = Object.freeze({
  read: 'personal_notifications_read_internal', claim: 'personal_notifications_claim_internal',
  report: 'personal_notifications_report_internal', open: 'personal_notifications_open_internal',
});
const MESSAGES = Object.freeze({ completed: '任务已完成', failed: '任务未完成', action_required: '任务需要确认' });
const PROFILE = /^[a-f0-9]{32}$/;
const EVENT = /^[a-f0-9]{64}$/;
const IDENTIFIER = /^[a-f0-9]{32}$/;
const ACTIVATION = /^cilai-personal-notification:([a-f0-9]{32})$/;
const SAFE_REASONS = new Set([
  'personal_notification_profile_changed', 'personal_notification_epoch_changed',
  'personal_notification_event_expired', 'personal_notification_disabled',
  'personal_notification_preferences_changed', 'personal_notification_delivery_limit',
  'personal_notification_not_found', 'personal_notification_target_missing',
  'personal_data_maintenance_active', 'personal_notification_outcome_frozen',
  'personal_notification_paused',
]);

function reason(error) {
  const text = String(error?.message ?? error);
  for (const code of SAFE_REASONS) if (text.split(/[^a-z_]+/).includes(code)) return code;
  return 'personal_notification_unavailable';
}

function validateSnapshot(value, profileId) {
  if (!value || value.profileId !== profileId || typeof value.sourceEpoch !== 'string'
    || !value.sourceEpoch || value.sourceEpoch.length > 128 || !Array.isArray(value.events)
    || value.events.length > 50000 || !value.preferences || typeof value.preferences.enabled !== 'boolean') {
    throw new Error('personal_notification_response_invalid');
  }
  for (const item of value.events) {
    if (!item || !EVENT.test(item.eventId) || !Object.hasOwn(MESSAGES, item.kind)
      || typeof item.enabledForDelivery !== 'boolean' || !Number.isSafeInteger(item.occurredAt)
      || (item.validUntil !== null && !Number.isSafeInteger(item.validUntil))) {
      throw new Error('personal_notification_response_invalid');
    }
  }
  return value;
}

function createPersonalNotifications({
  Notification, service, getWindow, navigate, platform = process.platform, headless = false,
  setTimer = setTimeout, clearTimer = clearTimeout, now = Date.now, outcomeTimeoutMs = 5000, requestTimeoutMs = 5000,
}) {
  if (!service || typeof service.call !== 'function' || typeof navigate !== 'function') {
    throw new Error('personal_notification_configuration_invalid');
  }
  let profileId = null;
  let generation = 0;
  let sourceEpoch = null;
  let seen = new Set();
  let timer = null;
  let active = false;
  let paused = false;
  let inFlight = null;
  let enabled = false;
  let historyAttached = false;
  let lastReason = null;
  let lastOutcome = null;
  let failures = 0;
  let pendingActivation = null;
  const notifications = new Map();
  const opening = new Set();

  function call(command, args) {
    let timeout;
    return Promise.race([
      Promise.resolve().then(() => service.call(command, args)),
      new Promise((_, reject) => { timeout = setTimer(() => reject(new Error('personal_notification_request_unconfirmed')), requestTimeoutMs); }),
    ]).finally(() => { if (timeout) clearTimer(timeout); });
  }

  function supported() {
    if (headless) return false;
    try { return typeof Notification?.isSupported === 'function' && Notification.isSupported(); }
    catch { return false; }
  }

  function status() {
    const available = supported();
    return { supported: available, active: active && !paused && enabled && available,
      reasonCode: headless ? 'notification_headless' : paused ? 'personal_notification_paused' : !available ? 'notification_unsupported' : lastReason,
      lastOutcome, profileId };
  }

  function closeOwned() {
    for (const notification of notifications.values()) {
      try { notification.close(); } catch { lastReason = 'notification_close_unconfirmed'; }
    }
    notifications.clear();
  }

  function current(expectedGeneration, expectedProfile) {
    return active && !paused && generation === expectedGeneration && profileId === expectedProfile;
  }

  async function open(notificationId) {
    if (!active || paused || !profileId || !IDENTIFIER.test(notificationId) || opening.has(notificationId)) return;
    const owner = profileId;
    const version = generation;
    opening.add(notificationId);
    try {
      const target = await call(PRIVATE_COMMANDS.open, { request: { profileId: owner, notificationId } });
      if (!current(version, owner)) throw new Error('personal_notification_profile_changed');
      if (!target || !['run', 'automation', 'application'].includes(target.kind)) throw new Error('personal_notification_response_invalid');
      const window = getWindow?.();
      if (!window || window.isDestroyed()) throw new Error('personal_notification_window_unavailable');
      if (window.isMinimized()) window.restore();
      window.show(); window.focus();
      await navigate({ profileId: owner, notificationId, target });
    } catch (error) { lastReason = reason(error); }
    finally { opening.delete(notificationId); }
  }

  function attach(notification) {
    if (!IDENTIFIER.test(notification?.id) || notifications.has(notification.id)) return;
    const owner = profileId;
    notification.on('click', () => { if (owner === profileId) void open(notification.id); });
    notifications.set(notification.id, notification);
  }

  async function restoreHistory(version, owner) {
    if (historyAttached || platform !== 'darwin') return;
    if (typeof Notification.getHistory !== 'function') {
      lastReason = 'notification_history_not_supported'; return;
    }
    historyAttached = true;
    try {
      const history = await Notification.getHistory();
      if (!current(version, owner)) return;
      if (!Array.isArray(history)) throw new Error('personal_notification_response_invalid');
      for (const notification of history) attach(notification);
    } catch { lastReason = 'notification_history_unavailable'; }
  }

  function nativeOptions(delivery, sound) {
    const options = { id: delivery.notificationId,
      groupId: `cilai-${createHash('sha256').update(delivery.profileId).digest('hex').slice(0, 16)}`,
      title: '词来个人工作台', body: MESSAGES[delivery.event.kind], silent: !sound, hasReply: false };
    if (platform === 'win32') {
      options.toastXml = `<toast launch="cilai-personal-notification:${delivery.notificationId}"><visual><binding template="ToastGeneric"><text>词来个人工作台</text><text>${MESSAGES[delivery.event.kind]}</text></binding></visual>${sound ? '' : '<audio silent="true"/>'}</toast>`;
    }
    return options;
  }

  function show(delivery, sound) {
    return new Promise(resolve => {
      let settled = false;
      let timeout;
      const finish = (outcome, reasonCode) => {
        if (settled) return;
        settled = true;
        if (timeout) clearTimer(timeout);
        resolve({ outcome, reasonCode });
      };
      try {
        const notification = new Notification(nativeOptions(delivery, sound));
        if (notification.id !== delivery.notificationId) throw new Error('notification_identifier_invalid');
        attach(notification);
        notification.once('show', () => finish('shown', null));
        notification.once('failed', () => finish('failed', 'notification_native_failed'));
        timeout = setTimer(() => finish('unknown', 'notification_outcome_unconfirmed'), outcomeTimeoutMs);
        notification.show();
      } catch { finish('failed', 'notification_native_failed'); }
    });
  }

  async function deliver(item, snapshot, version, owner) {
    if (!current(version, owner) || !enabled || !supported()) return;
    const claimed = await call(PRIVATE_COMMANDS.claim, { request: {
      profileId: owner, sourceEpoch: snapshot.sourceEpoch, eventId: item.eventId,
    } });
    const delivery = claimed?.delivery;
    if (typeof claimed?.mayDeliver !== 'boolean' || !delivery || delivery.profileId !== owner
      || delivery.event?.eventId !== item.eventId || !IDENTIFIER.test(delivery.notificationId)
      || !Object.hasOwn(MESSAGES, delivery.event.kind) || typeof claimed.sound !== 'boolean') {
      throw new Error('personal_notification_response_invalid');
    }
    if (!claimed.mayDeliver) return;
    let receipt;
    if (!current(version, owner) || !enabled) receipt = { outcome: 'unknown', reasonCode: 'notification_profile_changed' };
    else receipt = await show(delivery, claimed.sound);
    lastOutcome = receipt.outcome;
    try {
      await call(PRIVATE_COMMANDS.report, { request: {
        profileId: owner, notificationId: delivery.notificationId, ...receipt,
      } });
    } catch (error) { lastReason = reason(error); }
  }

  async function observe() {
    const owner = profileId;
    const version = generation;
    if (!active || paused || !owner) return;
    const snapshot = validateSnapshot(await call(PRIVATE_COMMANDS.read, {}), owner);
    if (!current(version, owner)) return;
    enabled = snapshot.preferences.enabled;
    if (!enabled) closeOwned();
    else if (supported()) await restoreHistory(version, owner);
    if (!current(version, owner)) return;
    const ids = new Set(snapshot.events.map(item => item.eventId));
    if (sourceEpoch !== snapshot.sourceEpoch) {
      sourceEpoch = snapshot.sourceEpoch; seen = ids; return;
    }
    const previous = seen;
    seen = new Set([...previous].filter(id => ids.has(id)));
    for (const item of snapshot.events) {
      if (previous.has(item.eventId)) continue;
      if (!item.enabledForDelivery || !enabled || (item.validUntil !== null && item.validUntil <= now())) {
        seen.add(item.eventId); continue;
      }
      if (!current(version, owner)) return;
      try { await deliver(item, snapshot, version, owner); }
      catch (error) {
        if (profileId === owner && sourceEpoch === snapshot.sourceEpoch
          && reason(error) !== 'personal_data_maintenance_active') seen.add(item.eventId);
        throw error;
      }
      if (profileId === owner && sourceEpoch === snapshot.sourceEpoch) seen.add(item.eventId);
      if (!current(version, owner)) return;
    }
  }

  function schedule() {
    if (!active || paused || timer) return;
    timer = setTimer(() => { timer = null; void refresh().finally(schedule); }, failures ? Math.min(30000, 2000 * (2 ** failures)) : 1000);
  }

  function refresh() {
    if (inFlight) return inFlight;
    inFlight = observe().then(() => { failures = 0; }).catch(error => {
      failures = Math.min(4, failures + 1); lastReason = reason(error);
    }).finally(() => { inFlight = null; });
    return inFlight;
  }

  function setProfile(next) {
    if (next !== null && (typeof next !== 'string' || !PROFILE.test(next))) throw new Error('personal_notification_profile_invalid');
    if (profileId === next) return;
    generation++; closeOwned(); profileId = next; sourceEpoch = null; seen = new Set(); enabled = false;
    historyAttached = false; lastReason = null; lastOutcome = null;
    consumeActivation();
  }

  function start() {
    if (active) return;
    active = true; paused = false;
    if (!headless && platform === 'win32' && typeof Notification?.handleActivation === 'function') {
      Notification.handleActivation(details => {
        const match = typeof details?.arguments === 'string' && ACTIVATION.exec(details.arguments);
        if (details?.type === 'click' && match) queueActivation(match[1]);
      });
    }
    void refresh().finally(schedule);
    consumeActivation();
  }

  async function pause() {
    paused = true; generation++;
    if (timer) clearTimer(timer);
    timer = null;
    await inFlight;
    closeOwned();
  }

  function resume() { if (!active) return; paused = false; void refresh().finally(schedule); consumeActivation(); }
  async function stop() { await pause(); active = false; sourceEpoch = null; seen.clear(); }
  function activation(launchInfo) {
    if (platform === 'darwin' && IDENTIFIER.test(launchInfo?.identifier)) queueActivation(launchInfo.identifier);
  }

  function queueActivation(identifier) { pendingActivation = identifier; consumeActivation(); }
  function consumeActivation() {
    if (!active || paused || !profileId || !pendingActivation) return;
    const identifier = pendingActivation;
    pendingActivation = null;
    void open(identifier);
  }

  async function preferences(command, args = {}) {
    if (!['personal_notification_preferences_get', 'personal_notification_preferences_save'].includes(command)) throw new Error('personal_notification_command_invalid');
    const owner = profileId;
    const version = generation;
    if (!owner) throw new Error('personal_notification_profile_invalid');
    if (!active || paused) throw new Error('personal_notification_paused');
    const result = await call(command, args);
    if (!current(version, owner) || result?.profileId !== owner) throw new Error('personal_notification_profile_changed');
    if (result?.preferences?.enabled === false) { enabled = false; closeOwned(); }
    await refresh();
    return { ...result, system: status() };
  }

  return { start, stop, pause, resume, setProfile, refresh, status, activation, preferences };
}

module.exports = { createPersonalNotifications, PRIVATE_COMMANDS };
