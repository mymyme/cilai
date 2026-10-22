const path = require("node:path");

function requestOf(args, keys) {
  const value = args?.request;
  if (!value || typeof value !== "object" || Array.isArray(value)
      || Object.keys(value).sort().join(",") !== keys.sort().join(",")
      || Object.values(value).some(item => typeof item !== "string" || !item.length || item.length > 256)) {
    throw new Error("personal_data_request_invalid");
  }
  return { ...value };
}

async function preparePersonalRestore({ args, window, dialog, service }) {
  const request = requestOf(args, ["clientOperationId"]);
  const selected = await dialog.showOpenDialog(window, {
    title: "选择资料备份", properties: ["openFile"], filters: [{ name: "资料备份", extensions: ["zip"] }],
  });
  if (selected.canceled) return null;
  if (!Array.isArray(selected.filePaths) || selected.filePaths.length !== 1
      || typeof selected.filePaths[0] !== "string" || !path.isAbsolute(selected.filePaths[0])
      || path.extname(selected.filePaths[0]).toLowerCase() !== ".zip") {
    throw new Error("personal_data_archive_invalid");
  }
  return service.call("personal_data_restore_prepare", { request: { ...request, archivePath: selected.filePaths[0] } });
}

async function planPersonalWorkspaceRestore({ args, window, dialog, service }) {
  const request = requestOf(args, ["operationId", "previewDigest", "clientOperationId"]);
  const selected = await dialog.showOpenDialog(window, {
    title: "选择恢复位置", properties: ["openDirectory", "createDirectory"],
  });
  if (selected.canceled) return null;
  if (!Array.isArray(selected.filePaths) || selected.filePaths.length !== 1
      || typeof selected.filePaths[0] !== "string" || !path.isAbsolute(selected.filePaths[0])) {
    throw new Error("personal_data_workspace_destination_invalid");
  }
  return service.call("personal_data_restore_workspace_plan", {
    request: { ...request, destinationParent: selected.filePaths[0] },
  });
}

async function runPersonalMaintenance({ command, args, closeApplications, service }) {
  const keys = ["operationId", "previewDigest"];
  if (command !== "personal_memory_reset_confirm") keys.push("clientOperationId");
  if (command === "personal_data_restore_apply") keys.push("expectedCurrentProfileId");
  else if (!["personal_data_backup_capture", "personal_storage_execute", "personal_memory_reset_confirm"].includes(command)) throw new Error("personal_data_command_not_allowed");
  const request = requestOf(args, keys);
  await closeApplications();
  return service.call(command, { request });
}

async function exportPersonalBackup({ args, window, dialog, service }) {
  const request = requestOf(args, ["operationId"]);
  const selected = await dialog.showSaveDialog(window, {
    title: "导出资料备份", defaultPath: "词来个人资料.zip", filters: [{ name: "资料备份", extensions: ["zip"] }],
  });
  if (selected.canceled || !selected.filePath) return { saved: false };
  if (!path.isAbsolute(selected.filePath)) throw new Error("personal_data_export_destination_invalid");
  return service.call("personal_data_backup_export", { operationId: request.operationId, destinationPath: selected.filePath });
}

async function runPersonalApplicationChange({ command, args, closeApplications, service }) {
  const expected = {
    personal_capability_upgrade: ["creationProjectId", "expectedRevision", "packageDigest", "confirmedPermissions", "clientOperationId"],
    personal_package_import_commit: ["importId", "previewDigest", "confirmedPermissions", "clientOperationId"],
    personal_application_data_rollback: ["operationId", "previewDigest"],
  }[command];
  const request = args?.request;
  if (!expected || !request || typeof request !== "object" || Array.isArray(request)
      || Object.keys(request).sort().join(",") !== expected.sort().join(",")) throw new Error("personal_application_data_request_invalid");
  await closeApplications();
  return service.call(command, { request });
}

async function restartAfterPersonalRestore({ service, app }) {
  const status = await service.call("app_server_status", {});
  if (status?.personalDataState !== "restart_required") throw new Error("personal_data_restart_not_ready");
  app.relaunch();
  app.quit();
  return null;
}

module.exports = { preparePersonalRestore, planPersonalWorkspaceRestore, runPersonalMaintenance, runPersonalApplicationChange, exportPersonalBackup, restartAfterPersonalRestore };
