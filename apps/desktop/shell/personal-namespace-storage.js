const fs = require("node:fs/promises");
const path = require("node:path");
const { APPLICATION_ID, rejectLinkedAncestors } = require("./personal-isolation");

const PROFILE = /^[a-f0-9]{32}$/;
const MAX_ENTRIES = 100_000;
const MAX_DEPTH = 32;
const fail = code => { throw new Error(code); };
const PUBLIC_REASONS = new Set([
  "personal_storage_inventory_limit", "personal_storage_link_not_scanned", "personal_storage_size_out_of_range",
  "personal_storage_special_file_not_scanned", "personal_storage_inventory_changed", "personal_storage_inventory_unavailable",
  "personal_storage_profile_changed", "personal_storage_browser_scope_invalid", "personal_storage_cache_size_invalid",
]);
const publicReason = (error, otherwise) => PUBLIC_REASONS.has(error?.message) ? error.message : otherwise;

function verifiedProfile(getVerifiedProfileId) {
  try {
    const profileId = getVerifiedProfileId();
    if (typeof profileId !== "string" || !PROFILE.test(profileId)) fail("personal_storage_profile_unavailable");
    return profileId;
  } catch {
    fail("personal_storage_profile_unavailable");
  }
}

function pathsOf(app) {
  let paths;
  try {
    const userData = app.getPath("userData");
    if (typeof userData !== "string" || !path.isAbsolute(userData) || path.resolve(userData) !== userData
        || path.basename(userData) !== APPLICATION_ID) fail("personal_storage_namespace_invalid");
    rejectLinkedAncestors(userData);
    paths = { userData, logs: app.getPath("logs"), sessionData: app.getPath("sessionData") };
    if (Object.values(paths).some(value => typeof value !== "string" || !path.isAbsolute(value) || path.resolve(value) !== value)
        || path.basename(paths.userData) !== APPLICATION_ID
        || paths.logs !== path.join(paths.userData, "logs")
        || paths.sessionData !== path.join(paths.userData, "browser")) fail("personal_storage_namespace_invalid");
    for (const directory of Object.values(paths)) rejectLinkedAncestors(directory);
  } catch {
    fail("personal_storage_namespace_invalid");
  }
  return paths;
}

function sameFile(before, after) {
  return before.dev === after.dev && before.ino === after.ino && before.mode === after.mode
    && before.size === after.size && before.mtimeNs === after.mtimeNs && before.ctimeNs === after.ctimeNs;
}

async function inspectDirectory(directory) {
  let entries = 0;
  let bytes = 0n;
  let files = 0;
  async function inspect(target, depth) {
    if (depth > MAX_DEPTH || ++entries > MAX_ENTRIES) fail("personal_storage_inventory_limit");
    const before = await fs.lstat(target, { bigint: true });
    if (before.isSymbolicLink()) fail("personal_storage_link_not_scanned");
    if (before.isDirectory()) {
      for (const name of await fs.readdir(target)) await inspect(path.join(target, name), depth + 1);
    } else if (before.isFile()) {
      bytes += before.size;
      files++;
      if (bytes > BigInt(Number.MAX_SAFE_INTEGER)) fail("personal_storage_size_out_of_range");
    } else {
      fail("personal_storage_special_file_not_scanned");
    }
    if (!sameFile(before, await fs.lstat(target, { bigint: true }))) fail("personal_storage_inventory_changed");
  }
  try {
    const root = await fs.lstat(directory);
    if (!root.isDirectory() || root.isSymbolicLink()) fail("personal_storage_inventory_unavailable");
  } catch (error) {
    if (error.code === "ENOENT") return { logicalBytes: 0, fileCount: 0, reason: null };
    return { logicalBytes: null, fileCount: null, reason: "personal_storage_inventory_unavailable" };
  }
  try {
    rejectLinkedAncestors(directory);
    await inspect(directory, 0);
    return { logicalBytes: Number(bytes), fileCount: files, reason: null };
  } catch (error) {
    const reason = publicReason(error, "personal_storage_inventory_unavailable");
    return { logicalBytes: null, fileCount: null, reason };
  }
}

function item(category, scope, observation) {
  return {
    itemId: `shell:${scope}`, category, displayPath: scope, managedRelativePath: null,
    logicalBytes: observation.logicalBytes, fileCount: observation.fileCount, snapshotDigest: null,
    cleanupAllowed: false,
    reasons: ["personal_namespace_current_directory_protected", ...(observation.reason ? [observation.reason] : [])], references: [],
  };
}

function existingBrowserSession(browser, profileId, sessions) {
  const partition = `persist:ai-browser-${profileId}`;
  if (browser.profileId !== profileId || !browser.isPartition(partition)) fail("personal_storage_profile_changed");
  let active = null;
  for (const tab of browser.tabs.values()) {
    const contents = tab.contents;
    if (contents.isDestroyed()) continue;
    const current = contents.session;
    if (active && current !== active) fail("personal_storage_browser_scope_invalid");
    active = current;
  }
  if (active) sessions.set(profileId, active);
  return active ?? sessions.get(profileId) ?? null;
}

async function browserCache(browser, profileId, directory, sessions) {
  try {
    const session = existingBrowserSession(browser, profileId, sessions);
    if (!session) return { logicalBytes: null, fileCount: null, reason: "personal_storage_browser_not_initialized", state: "not_scanned" };
    const storage = session.storagePath;
    if (typeof storage !== "string" || !path.isAbsolute(storage) || path.resolve(storage) !== storage
        || storage !== path.join(directory, "Partitions", `ai-browser-${profileId}`)) fail("personal_storage_browser_scope_invalid");
    rejectLinkedAncestors(storage);
    const size = await session.getCacheSize();
    if (!Number.isSafeInteger(size) || size < 0) fail("personal_storage_cache_size_invalid");
    return { logicalBytes: size, fileCount: null, reason: null };
  } catch (error) {
    const reason = publicReason(error, "personal_storage_cache_unavailable");
    return { logicalBytes: null, fileCount: null, reason };
  }
}

function createPersonalNamespaceStorage({ app, browser, getVerifiedProfileId, service, closeApplications }) {
  const sessions = new Map();
  const cleanup = require("./personal-namespace-cleanup").createPersonalNamespaceCleanup({
    app, browser, service, closeApplications,
    context: () => {
      const profileId = verifiedProfile(getVerifiedProfileId);
      if (browser.profileId !== profileId) fail("personal_storage_profile_changed");
      return { profileId, paths: pathsOf(app) };
    },
    existingSession: profileId => existingBrowserSession(browser, profileId, sessions),
  });
  return {
    prepare: args => cleanup.prepare(args),
    execute: args => cleanup.execute(args),
    finish: args => cleanup.finish(args),
    perform: args => cleanup.perform(args),
    async inventory(...args) {
      if (args.length) fail("personal_storage_request_invalid");
      const profileId = verifiedProfile(getVerifiedProfileId);
      const paths = pathsOf(app);
      const observations = await Promise.all([
        inspectDirectory(paths.logs),
        inspectDirectory(path.join(paths.userData, "test-diagnostics")),
        browserCache(browser, profileId, paths.sessionData, sessions),
      ]);
      const retainedLogSegments = await cleanup.retainedSegments();
      if (verifiedProfile(getVerifiedProfileId) !== profileId || browser.profileId !== profileId
          || JSON.stringify(pathsOf(app)) !== JSON.stringify(paths)) fail("personal_storage_profile_changed");
      const items = [
        item("logs", "namespace/logs", observations[0]),
        item("logs", "namespace/test-diagnostics", observations[1]),
        item("cache", "browser/http-cache", observations[2]),
      ];
      const issues = observations.map(value => value.reason).filter(Boolean);
      if (retainedLogSegments.some(value => value.reason)) issues.push("personal_namespace_segment_unavailable");
      const knownLogicalBytes = items.reduce((total, value) => total + (value.logicalBytes ?? 0), 0)
        + retainedLogSegments.reduce((total, value) => total + (value.logicalBytes ?? 0), 0);
      if (!Number.isSafeInteger(knownLogicalBytes)) fail("personal_storage_size_out_of_range");
      return {
        profileId, scannedAt: Date.now(), observation: "live_metadata", sizeComplete: issues.length === 0,
        knownLogicalBytes,
        coverage: items.map((value, index) => ({ category: value.category, scope: value.displayPath,
          state: observations[index].state ?? (observations[index].reason ? "unavailable" : "covered"), reason: observations[index].reason })),
        items, issues, retainedLogSegments,
      };
    },
  };
}

module.exports = { createPersonalNamespaceStorage };
