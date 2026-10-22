const fs = require("node:fs/promises");
const path = require("node:path");
const { randomUUID } = require("node:crypto");
const { rejectLinkedAncestors } = require("./personal-isolation");

const HEX = /^[a-f0-9]{32}$/;
const FILES = Object.freeze({
  host_service_current: { kind: "host_service_logs", source: "logs/host-service.log", name: "host-service.log" },
  host_service_previous: { kind: "host_service_logs", source: "logs/host-service.log.1", name: "host-service.log.1" },
  test_diagnostics_current: { kind: "test_diagnostics_logs", source: "test-diagnostics/events.log", name: "events.log" },
  test_diagnostics_previous: { kind: "test_diagnostics_logs", source: "test-diagnostics/events.log.1", name: "events.log.1" },
});
const KINDS = new Set(["host_service_logs", "test_diagnostics_logs", "browser_http_cache"]);
const fail = code => { throw new Error(code); };

function exact(value, keys) {
  if (!value || typeof value !== "object" || Array.isArray(value)
      || Object.keys(value).sort().join(",") !== [...keys].sort().join(",")) fail("personal_namespace_request_invalid");
  return value;
}
function confirmation(args) {
  const value = exact(args?.request, ["operationId", "previewDigest"]);
  if (Object.values(value).some(s => typeof s !== "string" || !s.length || s.length > 256)) fail("personal_namespace_request_invalid");
  return { ...value };
}
function sameSnapshot(left, right) {
  return Object.keys(left).length === Object.keys(right).length && Object.keys(left).every(key => left[key] === right[key]);
}
async function snapshot(file, target) {
  rejectLinkedAncestors(path.dirname(target));
  let metadata;
  try { metadata = await fs.lstat(target, { bigint: true }); }
  catch (error) { if (error.code !== "ENOENT") throw error; }
  if (metadata && (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > BigInt(Number.MAX_SAFE_INTEGER))) {
    fail("personal_namespace_log_file_invalid");
  }
  return { file, exists: Boolean(metadata), device: metadata?.dev.toString() ?? null,
    inode: metadata?.ino.toString() ?? null, mode: metadata?.mode.toString() ?? null,
    logicalBytes: metadata ? Number(metadata.size) : null,
    modifiedNanoseconds: metadata?.mtimeNs.toString() ?? null, changedNanoseconds: metadata?.ctimeNs.toString() ?? null };
}
function segmentPath(root, segmentId) {
  if (!HEX.test(segmentId)) fail("personal_namespace_request_invalid");
  const directory = path.join(root, "personal-storage-log-segments", segmentId);
  rejectLinkedAncestors(directory);
  return directory;
}
async function segmentManifest(root, segmentId, profileId) {
  const directory = segmentPath(root, segmentId);
  const target = path.join(directory, "segment.json");
  const metadata = await fs.lstat(target);
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > 16384) fail("personal_namespace_segment_invalid");
  const value = exact(JSON.parse(await fs.readFile(target, "utf8")), ["schemaVersion", "segmentId", "profileId", "createdAt"]);
  if (value.schemaVersion !== 1 || value.segmentId !== segmentId || value.profileId !== profileId || !Number.isSafeInteger(value.createdAt)) {
    fail("personal_namespace_segment_profile_mismatch");
  }
  const allowed = new Set(["segment.json", ...Object.values(FILES).map(value => value.name)]);
  if ((await fs.readdir(directory)).some(name => !allowed.has(name))) fail("personal_namespace_segment_invalid");
  return directory;
}

function createPersonalNamespaceCleanup({ app, browser, service, closeApplications, context, existingSession }) {
  const instanceId = randomUUID();
  const attempts = new Map();
  const shell = () => ({ shellPid: process.pid, shellInstanceId: instanceId });
  const call = (command, request) => service.call(command, { request });
  async function acknowledge(entry) {
    const params = entry.pendingAck;
    if (!params) return;
    const receipt = await call("personal_namespace_storage_action_ack_internal", params);
    if (!receipt || receipt.actionId !== params.actionId || receipt.state !== "acknowledged" || receipt.outcome !== params.outcome
        || receipt.afterCacheBytes !== params.afterCacheBytes || receipt.reasonCode !== params.reasonCode) fail("personal_namespace_action_unconfirmed");
    entry.pendingAck = null;
    entry.pending = false;
  }

  function pauseLogs(kinds) {
    const leases = [];
    try {
      if (kinds.includes("host_service_logs") && service.diagnosticsLog) leases.push(service.diagnosticsLog.suspendForStorage());
      if (kinds.includes("test_diagnostics_logs") && service.testDiagnostics) leases.push(service.testDiagnostics.suspendForStorage());
    } catch (error) {
      for (const lease of leases) lease.resume();
      throw error;
    }
    return leases;
  }
  async function freezeLogs(current, kinds) {
    const segmentId = randomUUID().replaceAll("-", "");
    const leases = pauseLogs(kinds);
    let moved = false;
    try {
      for (const lease of leases) lease.sealPending();
      await call("personal_namespace_storage_segment_create_internal", { segmentId, ...shell() });
      const target = segmentPath(current.paths.userData, segmentId);
      for (const [file, spec] of Object.entries(FILES)) {
        if (!kinds.includes(spec.kind)) continue;
        const source = path.join(current.paths.userData, spec.source);
        const before = await snapshot(file, source);
        if (before.exists) {
          await fs.rename(source, path.join(target, spec.name));
          moved = true;
        }
      }
      await call("personal_namespace_storage_segment_sync_internal", { segmentId, ...shell() });
      return segmentId;
    } finally {
      if (moved) for (const lease of leases) lease.rebase();
      for (const lease of leases) lease.resume();
    }
  }
  async function logSnapshots(current, kinds, segmentId) {
    if (!segmentId) return [];
    const directory = await segmentManifest(current.paths.userData, segmentId, current.profileId);
    return Promise.all(Object.entries(FILES).filter(([, spec]) => kinds.includes(spec.kind))
      .map(([file, spec]) => snapshot(file, path.join(directory, spec.name))));
  }
  function browserSession(current) {
    const session = existingSession(current.profileId);
    if (!session) fail("personal_storage_browser_not_initialized");
    const expected = path.join(current.paths.sessionData, "Partitions", `ai-browser-${current.profileId}`);
    if (session.storagePath !== expected) fail("personal_namespace_browser_scope_invalid");
    rejectLinkedAncestors(expected);
    return session;
  }
  function sameContext(current) {
    const next = context();
    if (next.profileId !== current.profileId || JSON.stringify(next.paths) !== JSON.stringify(current.paths)) fail("personal_storage_profile_changed");
  }
  function release(operationId) {
    const entry = attempts.get(operationId);
    if (!entry || entry.released) return;
    if (entry.pending) fail("personal_namespace_action_unconfirmed");
    for (const lease of entry.logs) lease.rebase();
    for (const lease of entry.logs) lease.resume();
    entry.browser?.resume();
    entry.released = true;
  }
  function releaseConfirmed(request, result) {
    if (result?.maintenanceError) return;
    const operation = result?.operation;
    if (!operation || operation.operationId !== request.operationId || operation.previewDigest !== request.previewDigest
        || !["completed", "incomplete", "cancelled"].includes(operation.state) || !Array.isArray(operation.actions)
        || operation.actions.some(action => action.state === "started")) fail("personal_namespace_action_unconfirmed");
    const entry = attempts.get(request.operationId);
    if (entry) entry.pending = false;
    release(request.operationId);
  }

  return {
    async retainedSegments() {
      const current = context(), directory = path.join(current.paths.userData, "personal-storage-log-segments");
      rejectLinkedAncestors(directory);
      let names;
      try { names = await fs.readdir(directory); } catch (error) { if (error.code === "ENOENT") return []; throw error; }
      if (names.length > 10000) fail("personal_storage_inventory_limit");
      const result = [];
      for (const segmentId of names) {
        if (!HEX.test(segmentId)) fail("personal_namespace_segment_invalid");
        try {
          const snapshots = await logSnapshots(current, ["host_service_logs", "test_diagnostics_logs"], segmentId);
          const present = snapshots.filter(value => value.exists);
          if (present.length) result.push({ segmentId, logicalBytes: present.reduce((n, value) => n + value.logicalBytes, 0), fileCount: present.length, cleanupAllowed: true, reason: null });
        } catch {
          result.push({ segmentId, logicalBytes: null, fileCount: null, cleanupAllowed: false, reason: "personal_namespace_segment_unavailable" });
        }
      }
      sameContext(current);
      return result;
    },
    async prepare(args) {
      const request = exact(args?.request, ["clientOperationId", "itemKinds", "logSegmentId"]);
      if (typeof request.clientOperationId !== "string" || !request.clientOperationId.length || request.clientOperationId.length > 128
          || !Array.isArray(request.itemKinds) || !request.itemKinds.length || request.itemKinds.length > 3
          || new Set(request.itemKinds).size !== request.itemKinds.length || request.itemKinds.some(kind => !KINDS.has(kind))
          || (request.logSegmentId !== null && !HEX.test(request.logSegmentId))) fail("personal_namespace_request_invalid");
      const current = context();
      const hasLogs = request.itemKinds.some(kind => kind !== "browser_http_cache");
      if (!hasLogs && request.logSegmentId !== null) fail("personal_namespace_request_invalid");
      let browserSnapshot = null;
      if (request.itemKinds.includes("browser_http_cache")) {
        const session = browserSession(current), size = await session.getCacheSize();
        if (!Number.isSafeInteger(size) || size < 0) fail("personal_storage_cache_size_invalid");
        browserSnapshot = { partition: `persist:ai-browser-${current.profileId}`, storagePath: session.storagePath, cacheBytes: size };
      }
      const segmentId = hasLogs ? request.logSegmentId ?? await freezeLogs(current, request.itemKinds) : null;
      const logs = await logSnapshots(current, request.itemKinds, segmentId);
      sameContext(current);
      return call("personal_namespace_storage_prepare_internal", { clientOperationId: request.clientOperationId,
        itemKinds: request.itemKinds, ...shell(), namespacePath: current.paths.userData,
        logSegmentId: segmentId, logFiles: logs, browserSession: browserSnapshot });
    },
    async execute(args) {
      const request = confirmation(args);
      await closeApplications();
      const result = await call("personal_namespace_storage_execute_internal", request);
      releaseConfirmed(request, result);
      return result;
    },
    async finish(args) {
      const request = confirmation(args);
      const entry = attempts.get(request.operationId);
      if (entry?.pendingAck) await acknowledge(entry);
      const result = await call("personal_namespace_storage_finish", request);
      releaseConfirmed(request, result);
      return result;
    },
    async perform(started) {
      const current = context(), operation = started?.operation, permits = started?.permits;
      if (!operation || operation.profileId !== current.profileId || operation.namespacePath !== current.paths.userData
          || operation.shellOwner?.pid !== process.pid || operation.shellOwner?.instanceId !== instanceId
          || !Array.isArray(permits) || !permits.length || permits.length > 3 || attempts.has(operation.operationId)) {
        fail("personal_namespace_action_unconfirmed");
      }
      const entry = { logs: [], browser: null, pending: false, pendingAck: null, released: false };
      attempts.set(operation.operationId, entry);
      let session = null;
      if (permits.some(value => value.kind === "browser_http_cache")) {
        session = browserSession(current);
        if (!operation.browserSession || session.storagePath !== operation.browserSession.storagePath) fail("personal_namespace_browser_scope_invalid");
        entry.browser = browser.suspendForStorage(session);
        await entry.browser.drain();
      }
      for (const permit of permits) {
        const planned = operation.actions.find(value => value.actionId === permit.actionId && value.kind === permit.kind);
        if (!planned || planned.state !== "planned" || !KINDS.has(permit.kind) || JSON.stringify(permit.shellOwner) !== JSON.stringify(operation.shellOwner)
            || typeof permit.permitToken !== "string" || !permit.permitToken.length || permit.permitToken.length > 512) fail("personal_namespace_action_unconfirmed");
        sameContext(current);
        const base = { operationId: operation.operationId, actionId: permit.actionId, permitToken: permit.permitToken, shellOwner: permit.shellOwner };
        const executionPids = permit.kind === "browser_http_cache"
          ? [...new Set([process.pid, ...app.getAppMetrics().map(value => value.pid)])] : [process.pid];
        const begun = await call("personal_namespace_storage_action_begin_internal", { ...base, executionPids });
        if (!begun || begun.actionId !== permit.actionId || begun.kind !== permit.kind || begun.state !== "started") fail("personal_namespace_action_unconfirmed");
        entry.pending = true;
        let outcome = "succeeded", reasonCode = null, afterCacheBytes = null;
        try {
          if (permit.kind === "browser_http_cache") {
            if (await session.getCacheSize() !== operation.browserSession.cacheBytes) fail("personal_namespace_cache_changed");
            await session.clearCache();
            afterCacheBytes = await session.getCacheSize();
            if (!Number.isSafeInteger(afterCacheBytes) || afterCacheBytes < 0) fail("personal_namespace_cache_size_invalid");
          } else {
            const directory = await segmentManifest(current.paths.userData, operation.logSegmentId, current.profileId);
            const files = operation.logFiles.filter(value => FILES[value.file]?.kind === permit.kind);
            if (files.length !== 2) fail("personal_namespace_log_plan_invalid");
            for (const expected of files) {
              const observed = await snapshot(expected.file, path.join(directory, FILES[expected.file].name));
              if (!sameSnapshot(observed, expected)) fail("personal_namespace_log_changed");
            }
            for (const expected of files) {
              const target = path.join(directory, FILES[expected.file].name);
              if (!sameSnapshot(await snapshot(expected.file, target), expected)) fail("personal_namespace_log_changed");
              if (expected.exists) await fs.unlink(target);
            }
            await call("personal_namespace_storage_segment_sync_internal", { segmentId: operation.logSegmentId, ...shell() });
          }
        } catch {
          outcome = "failed";
          reasonCode = permit.kind === "browser_http_cache" ? "personal_namespace_storage_cache_failed" : "personal_namespace_storage_log_failed";
          afterCacheBytes = null;
        }
        entry.pendingAck = { ...base, outcome, reasonCode, afterCacheBytes };
        await acknowledge(entry);
      }
      return { acknowledged: true, operationId: operation.operationId };
    },
  };
}

module.exports = { createPersonalNamespaceCleanup };
