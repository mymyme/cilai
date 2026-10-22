import { createHash } from "node:crypto";
import { closeSync, existsSync, openSync, readSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { packageBundlePaths } from './package-output.mjs';
import { isMachO, verifyMacSignature, verifySignedMacPayload } from '../../../scripts/macos-binary-integrity.mjs';

function filesBelow(root, ignoreDsym = false) {
  if (!existsSync(root)) throw new Error(`产物来源不存在：${root}`);
  const files = [];
  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      if (ignoreDsym && entry.name.endsWith(".dSYM")) continue;
      const file = join(directory, entry.name);
      if (entry.isDirectory()) visit(file);
      else if (entry.isFile()) files.push(relative(root, file));
      else throw new Error(`产物来源包含非普通文件：${file}`);
    }
  };
  visit(root);
  if (files.length === 0) throw new Error(`产物来源为空：${root}`);
  return files;
}

function digest(file) {
  const hash = createHash("sha256");
  const fd = openSync(file, "r");
  const buffer = Buffer.allocUnsafe(1024 * 1024);
  try {
    let size;
    while ((size = readSync(fd, buffer, 0, buffer.length, null)) > 0) {
      hash.update(buffer.subarray(0, size));
    }
    return hash.digest("hex");
  } finally {
    closeSync(fd);
  }
}

function verifyFiles(sourceRoot, packagedRoot, label, ignoreDsym = false, signed = false, platform) {
  const files = filesBelow(sourceRoot, ignoreDsym);
  const packagedFiles = filesBelow(packagedRoot, ignoreDsym);
  const expected = new Set(files);
  for (const file of packagedFiles) {
    if (!expected.has(file)) throw new Error(`产物含有额外${label}：${file}`);
  }
  for (const file of files) {
    const source = join(sourceRoot, file);
    const packaged = join(packagedRoot, file);
    if (!existsSync(packaged) || !statSync(packaged).isFile()) {
      throw new Error(`产物缺少${label}：${file}`);
    }
    const immutable = file.split(/[\\/]/)[0] === "codex-runtime";
    if (statSync(source).size === 0 || statSync(packaged).size === 0) {
      throw new Error(`产物${label}与当前构建结果不一致：${file}`);
    }
    if (platform === 'darwin' && signed && !immutable && isMachO(source)) {
      verifySignedMacPayload(source, packaged);
      continue;
    }
    const windowsSignedExecutable = platform === 'win32' && signed && !immutable && /\.(?:exe|dll|node)$/i.test(file);
    if (!windowsSignedExecutable && (statSync(source).size !== statSync(packaged).size || digest(source) !== digest(packaged))) {
      throw new Error(`产物${label}与当前构建结果不一致：${file}`);
    }
    if (platform === 'darwin' && immutable && isMachO(packaged)) verifyMacSignature(packaged);
  }
  return files.length;
}

export function verifyPackageLayout({ output, platform, arch, productName, executableName = productName, stagedDir, frontendDir, signedSidecars = false }) {
  const { bundle, resources, executable } = packageBundlePaths({ output, platform, arch, productName, executableName });
  if (!existsSync(executable) || !statSync(executable).isFile()) {
    throw new Error(`产物缺少桌面主程序：${executable}`);
  }
  const sidecarCount = verifyFiles(stagedDir, join(resources, "sidecar"), "随包组件", true, signedSidecars, platform);
  const frontendCount = verifyFiles(frontendDir, join(resources, "frontend"), "前端");
  return { bundle, resources, executable, sidecarCount, frontendCount };
}
