const { randomUUID } = require("node:crypto");

function createPersonalApplicationClose({ getOwner, documents, emit, revokePageAi, timeoutMs = 10_000, detachTimeoutMs = 2_000 }) {
  let pending = null;
  let closing = false;
  return {
    async close() {
      if (closing) throw new Error("personal_application_close_busy");
      closing = true;
      try {
      if (revokePageAi) await revokePageAi();
      const owner = getOwner();
      if (!owner || owner.isDestroyed()) { documents.clear(); return; }
      if (!documents.frames(owner).length) { documents.clear(); return; }
      const token = randomUUID();
      await new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          if (pending?.token === token) pending = null;
          reject(new Error("personal_application_close_unconfirmed"));
        }, timeoutMs);
        pending = { token, owner, timer, resolve, reject };
        let sent = false;
        try { sent = Boolean(emit("personal-applications-close", { token })); } catch {}
        if (!sent) {
          clearTimeout(timer); pending = null;
          reject(new Error("personal_application_close_listener_unavailable"));
        }
      });
      const deadline = Date.now() + detachTimeoutMs;
      while (owner === getOwner() && !owner.isDestroyed() && documents.frames(owner).length) {
        if (Date.now() >= deadline) throw new Error("personal_application_frames_active");
        await new Promise(resolve => setTimeout(resolve, 20));
      }
      if (owner !== getOwner()) throw new Error("personal_application_close_owner_changed");
      documents.clear();
      } finally { closing = false; }
    },
    acknowledge(args, sender) {
      const entry = pending;
      if (!entry || !args || typeof args !== "object" || Array.isArray(args)
          || Object.keys(args).join(",") !== "token" || args.token !== entry.token
          || sender !== entry.owner || sender !== getOwner() || sender.isDestroyed()) throw new Error("personal_application_close_ack_invalid");
      pending = null; clearTimeout(entry.timer); entry.resolve();
      return null;
    },
  };
}

module.exports = { createPersonalApplicationClose };
