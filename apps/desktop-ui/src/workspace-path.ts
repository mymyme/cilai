/** Resolve a displayed file reference without granting access outside its workspace. */
export function workspaceRelativePath(href: string, workspaceRoot: string): string | null {
  let path = href.trim();
  if (!path || path.startsWith("#")) return null;
  const root = workspaceRoot.replaceAll("\\", "/").replace(/\/+$/, "");
  const windows = /^[a-z]:\//i.test(`${root}/`) || root.startsWith("//");
  path = path.split("#", 1)[0].split("?", 1)[0];
  try { path = decodeURIComponent(path); } catch { return null; }
  path = path.replaceAll("\\", "/");
  // Do not use URL.pathname here: URL parsing silently resolves traversal segments.
  if (/^file:\/\//i.test(path)) {
    const match = /^file:\/\/([^/]*)(\/.*)$/i.exec(path);
    if (!match || /[@:]/.test(match[1])) return null;
    path = match[1] && match[1].toLowerCase() !== "localhost" ? `//${match[1]}${match[2]}` : match[2];
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

export function workspacePathsEqual(first: string, second: string, root: string): boolean {
  const a = first.replaceAll("\\", "/"), b = second.replaceAll("\\", "/");
  return /^[a-z]:[\\/]/i.test(root) || /^[\\/]{2}/.test(root) ? a.toLowerCase() === b.toLowerCase() : a === b;
}
