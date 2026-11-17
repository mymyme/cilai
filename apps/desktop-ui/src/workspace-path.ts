export function workspaceAbsolutePath(workspaceRoot: string, relativePath: string, hostPlatform: string | null = null): string {
  const windows = hostPlatform === "windows" || (hostPlatform === null && /^(?:[a-z]:[\\/]|[\\/]{2})/i.test(workspaceRoot));
  if (!windows) return `${workspaceRoot.replace(/\/+$/, "")}/${relativePath}`;
  const root = workspaceRoot.replaceAll("/", "\\").replace(/\\+$/, "");
  return `${root}\\${relativePath.replaceAll("/", "\\")}`;
}

/** Resolve a displayed file reference without granting access outside its workspace. */
export function workspaceRelativePath(href: string, workspaceRoot: string): string | null {
  let path = normalizeWindowsFilePath(href.trim());
  if (!path || path.startsWith("#")) return null;
  const normalizedRoot = normalizeWindowsFilePath(workspaceRoot);
  if (normalizedRoot === null) return null;
  const root = normalizedRoot.replace(/\/+$/, "");
  const windows = /^[a-z]:\//i.test(`${root}/`) || root.startsWith("//");
  path = path.split("#", 1)[0].split("?", 1)[0];
  try { path = normalizeWindowsFilePath(decodeURIComponent(path)); } catch { return null; }
  if (!path) return null;
  // Do not use URL.pathname here: URL parsing silently resolves traversal segments.
  if (/^file:\/\//i.test(path)) {
    const match = /^file:\/\/([^/]*)(\/.*)$/i.exec(path);
    if (!match || /[@:]/.test(match[1])) return null;
    path = match[1] && match[1].toLowerCase() !== "localhost" ? `//${match[1]}${match[2]}` : match[2];
    path = normalizeWindowsFilePath(path);
    if (!path) return null;
  } else if (/^[a-z][a-z0-9+.-]*:/i.test(path) && !/^[a-z]:\//i.test(path)) {
    return null;
  }
  if (/[\u0000-\u001f\u007f]/.test(path) || path.split("/").includes("..")) return null;
  if (/^\/[a-z]:\//i.test(path)) path = path.slice(1);
  path = path.replace(/:\d+(?::\d+)?$/, "");
  const absolute = path.startsWith("/") || /^[a-z]:\//i.test(path);
  if (absolute) {
    if (!root && workspaceRoot !== "/") return null;
    const prefix = `${root}/`;
    if (!(windows ? path.toLowerCase().startsWith(prefix.toLowerCase()) : path.startsWith(prefix))) return null;
    path = path.slice(prefix.length);
  }
  path = path.replace(/^\.\//, "");
  if (!path || path.split("/").some((part) => !part || part === "." || part === "..")) return null;
  // A drive-relative path or alternate data stream is never a workspace file link.
  if (windows && path.includes(":")) return null;
  return path;
}

/** Rust canonical paths use the Win32 extended prefix; compare the same file spelling only. */
function normalizeWindowsFilePath(value: string): string | null {
  const path = value.replaceAll("\\", "/");
  if (path.startsWith("//?/")) {
    if (/^\/\/\?\/[a-z]:\//i.test(path)) return path.slice(4);
    if (/^\/\/\?\/UNC\/[^/]+\/[^/]+(?:\/|$)/i.test(path)) return `//${path.slice(8)}`;
    return null;
  }
  // Device namespaces are not workspace file references.
  return path.startsWith("//./") ? null : path;
}

export function workspacePathsEqual(first: string, second: string, root: string): boolean {
  const a = first.replaceAll("\\", "/"), b = second.replaceAll("\\", "/");
  return /^[a-z]:[\\/]/i.test(root) || /^[\\/]{2}/.test(root) ? a.toLowerCase() === b.toLowerCase() : a === b;
}
