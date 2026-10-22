function createPersonalNotificationNavigation({ documents, service, pageAi, getOwner, getProfile, emit, timeoutMs = 10000 }) {
  let ready = null;
  let pending = null;
  let stopped = false;
  const error = () => new Error('personal_notification_target_missing');
  function ownerIdentity() {
    const owner = getOwner(); const frame = owner?.mainFrame;
    if (!owner || owner.isDestroyed() || !frame || frame.detached) throw error();
    return { owner, frame, processId: frame.processId, routingId: frame.routingId };
  }
  function available(profileId) {
    if (stopped || ready?.profileId !== profileId || getProfile() !== profileId) return false;
    const owner = getOwner(); const frame = owner?.mainFrame;
    return Boolean(owner && !owner.isDestroyed() && frame && !frame.detached
      && ready.owner === owner && ready.frame === frame
      && ready.processId === frame.processId && ready.routingId === frame.routingId);
  }
  async function drain() {
    const item = pending;
    if (!item || item.running || !available(item.profileId)) return;
    item.running = true;
    try {
      const target = await service.call('personal_notifications_open_internal', { request: { profileId: item.profileId, notificationId: item.notificationId } });
      const location = target?.kind === 'application' ? await pageAi.notificationLocation(target) : {};
      if (pending !== item || !available(item.profileId)) throw error();
      if (emit('personal-notification-open', { profileId: item.profileId, notificationId: item.notificationId, target: { ...target, ...location } }) < 1) throw error();
      item.resolve();
    } catch (failure) { item.reject(failure); }
    finally { if (pending === item) pending = null; clearTimeout(item.timer); }
  }
  function invalidate() {
    ready = null;
    if (pending) { clearTimeout(pending.timer); pending.reject(error()); pending = null; }
  }
  return {
    handles: command => command === "personal_notification_navigation_ready",
    open({ profileId, notificationId }) {
      if (stopped || getProfile() !== profileId || !/^[a-f0-9]{32}$/.test(notificationId)) return Promise.reject(error());
      if (pending) { clearTimeout(pending.timer); pending.reject(error()); }
      return new Promise((resolve, reject) => {
        const item = { profileId, notificationId, resolve, reject, running: false };
        item.timer = setTimeout(() => { if (pending === item) { pending = null; item.reject(error()); } }, timeoutMs);
        pending = item;
        void drain().catch(reject);
      });
    },
    ready(event, args) {
      documents.verify(event);
      if (!args || Object.keys(args).join(',') !== 'profileId' || args.profileId !== getProfile()) throw error();
      ready = { ...ownerIdentity(), profileId: args.profileId };
      void drain().catch(() => { if (pending) { clearTimeout(pending.timer); pending.reject(error()); pending = null; } });
      return { ready: true };
    },
    invalidate,
    stop() { stopped = true; invalidate(); },
  };
}
module.exports = { createPersonalNotificationNavigation };
