const COMMANDS = new Set(["macos_disk_access_open_settings", "macos_disk_access_reveal_application"]);

function createMacosDiskAccess(getOwner, call) {
  return {
    handles: command => COMMANDS.has(command),
    async invoke(event, { command, args }) {
      const owner = getOwner();
      if (!owner || owner.isDestroyed() || event.sender !== owner || event.senderFrame !== owner.mainFrame || !event.senderFrame.url.startsWith("app://localhost/")) {
        throw new Error("磁盘访问设置只能从工作台主界面打开");
      }
      if (!COMMANDS.has(command) || !args || typeof args !== "object" || Array.isArray(args) || Object.keys(args).length) {
        throw new Error("磁盘访问设置不接受额外参数");
      }
      return call(command, {});
    },
  };
}

module.exports = { createMacosDiskAccess };
