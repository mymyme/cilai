const path = require("node:path");

function differentInstancePath(currentExecutable, additionalData) {
  const requestedExecutable = additionalData?.executablePath;
  if (typeof requestedExecutable !== "string" || !requestedExecutable.trim()) return null;
  if (path.normalize(requestedExecutable) === path.normalize(currentExecutable)) return null;
  return requestedExecutable;
}

module.exports = { differentInstancePath };
