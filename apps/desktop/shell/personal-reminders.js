const identityCommands = new Set(["app_server_restart", "personal_data_restore", "personal_profile_close"]);

function installPersonalReminders({ service, powerMonitor, Notification, publish, showWindow, log, notificationSettings }) {
  const shown = new Map();
  let session = null;
  let epoch = 0;
  let waking = false;
  let suspended = false;
  let stopped = false;
  let inFlight = false;
  let controls = 0;
  let lastError = null;

  function clearDesktop() {
    for (const notification of shown.values()) notification.close();
    shown.clear();
  }

  function report(runtimeSessionId, noticeId, outcome) {
    if (runtimeSessionId !== session || stopped) return;
    void service.call("ai_pa_reminder_result", { runtimeSessionId, noticeId, outcome })
      .then(() => {
        if (runtimeSessionId === session && !stopped) {
          publish("personal-reminders-changed", { runtimeSessionId });
        }
      })
      .catch(error => log(error));
  }

  function show(runtimeSessionId, notice, settings) {
    if (!settings.enabled) { report(runtimeSessionId, notice.id, "disabled"); return; }
    if (!Notification.isSupported()) {
      report(runtimeSessionId, notice.id, "unavailable");
      return;
    }
    try {
      const notification = new Notification({
        title: "待办提醒",
        body: "你有一项待办到达提醒时间。",
        silent: !settings.sound,
      });
      notification.on("click", () => {
        if (runtimeSessionId !== session || stopped || controls) return;
        showWindow();
        publish("personal-reminder-open", { runtimeSessionId, todoId: notice.todoId });
      });
      notification.on("failed", () => {
        shown.delete(notice.id);
        report(runtimeSessionId, notice.id, "failed");
      });
      notification.on("close", () => shown.delete(notice.id));
      if (shown.size >= 100) {
        const [id, oldest] = shown.entries().next().value;
        shown.delete(id);
        oldest.close();
      }
      shown.set(notice.id, notification);
      notification.show();
      report(runtimeSessionId, notice.id, "submitted");
    } catch (error) {
      shown.delete(notice.id);
      report(runtimeSessionId, notice.id, "failed");
      log(error);
    }
  }

  async function tick() {
    if (stopped || suspended || inFlight || controls || !service.child || service.updateHold || service.stopRequest) return;
    inFlight = true;
    const started = epoch;
    try {
      const result = await service.call("ai_pa_reminder_tick", {
        previousRuntimeSessionId: session, waking,
      });
      if (started !== epoch || stopped || suspended || controls) return;
      waking = false;
      lastError = null;
      const nextSession = result?.runtimeSessionId ?? null;
      if (nextSession !== session) clearDesktop();
      session = nextSession;
      if (result?.changed) publish("personal-reminders-changed", { runtimeSessionId: session });
      const settings = (result?.desktop?.length ?? 0) > 0 ? await notificationSettings() : null;
      if (started !== epoch || stopped || suspended || controls || nextSession !== session) return;
      for (const notice of result?.desktop ?? []) {
        const claimed = await service.call("ai_pa_reminder_claim", { runtimeSessionId: nextSession, noticeId: notice.id });
        if (started !== epoch || stopped || suspended || controls || nextSession !== session) return;
        if (claimed) show(nextSession, notice, settings);
      }
    } catch (error) {
      const reason = String(error?.message ?? error);
      if (lastError !== reason && started === epoch && !stopped) {
        lastError = reason;
        log(error);
      }
    } finally {
      inFlight = false;
    }
  }

  function suspend() {
    epoch += 1;
    suspended = true;
  }
  function resume() {
    epoch += 1;
    suspended = false;
    waking = true;
    void tick();
  }
  powerMonitor.on("suspend", suspend);
  powerMonitor.on("resume", resume);
  const timer = setInterval(() => void tick(), 5000);
  timer.unref();
  void tick();

  return {
    beginControl(command) {
      if (!identityCommands.has(command)) return null;
      epoch += 1;
      controls += 1;
      // Expire clicks from the previous identity while the host verifies the change.
      session = null;
      clearDesktop();
      return () => {
        controls -= 1;
        void tick();
      };
    },
    stop() {
      stopped = true;
      epoch += 1;
      clearInterval(timer);
      powerMonitor.off("suspend", suspend);
      powerMonitor.off("resume", resume);
      clearDesktop();
    },
  };
}

module.exports = { installPersonalReminders };
