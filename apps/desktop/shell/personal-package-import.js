const path = require("node:path");

function importTarget(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("personal_package_import_target_invalid");
  const keys = Object.keys(value).sort().join(",");
  if (value.mode === "new" && keys === "mode") return { mode: "new" };
  if (value.mode === "upgrade" && keys === "creationProjectId,expectedRevision,mode"
      && typeof value.creationProjectId === "string" && /^capability_[a-f0-9]{32}$/.test(value.creationProjectId)
      && Number.isSafeInteger(value.expectedRevision) && value.expectedRevision > 0) {
    return { mode: "upgrade", creationProjectId: value.creationProjectId, expectedRevision: value.expectedRevision };
  }
  throw new Error("personal_package_import_target_invalid");
}

async function preparePersonalPackageImport({ args, window, dialog, service }) {
  const request = args?.request;
  if (!request || Object.keys(request).sort().join(",") !== "clientOperationId,target"
      || typeof request.clientOperationId !== "string" || !/^[a-zA-Z0-9_-]{1,160}$/.test(request.clientOperationId)) {
    throw new Error("personal_package_import_request_invalid");
  }
  const target = importTarget(request.target);
  const picked = await dialog.showOpenDialog(window, {
    title: "选择个人安装包", properties: ["openFile"], filters: [{ name: "个人安装包", extensions: ["zip"] }],
  });
  if (picked.canceled) return null;
  if (!Array.isArray(picked.filePaths) || picked.filePaths.length !== 1
      || typeof picked.filePaths[0] !== "string" || !path.isAbsolute(picked.filePaths[0])
      || path.extname(picked.filePaths[0]).toLowerCase() !== ".zip") {
    throw new Error("personal_package_import_archive_invalid");
  }
  return service.call("personal_package_import_prepare", { request: {
    archivePath: picked.filePaths[0], target, clientOperationId: request.clientOperationId,
  } });
}

module.exports = { preparePersonalPackageImport };
