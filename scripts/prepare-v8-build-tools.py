import base64
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import urllib.parse
import urllib.request
import zipfile


def metadata(endpoint, parameters):
    query = urllib.parse.urlencode(parameters)
    url = (
        f"https://chrome-infra-packages.appspot.com/_ah/api/repo/v1/{endpoint}?{query}"
    )
    with urllib.request.urlopen(url, timeout=60) as response:
        return json.load(response)


def prepare_rust_sources(crate, source):
    print("Verifying pinned Chromium Rust sources", flush=True)
    repository, revision = source.rsplit("@", 1)
    relative = "chromium_crates_io/vendor"
    with urllib.request.urlopen(
        f"{repository}/+/{revision}/{relative}/?format=JSON", timeout=60
    ) as response:
        expected = json.loads(response.read().decode().split("\n", 1)[1])["id"]
    directory = crate / "third_party/rust" / relative
    if directory.exists():
        digest, count = directory_tree(directory)
        if digest.hex() != expected:
            raise ValueError(
                "existing Rust source tree conflicts with pinned dependency"
            )
        return {
            "name": "chromium-rust-vendor",
            "repository": repository,
            "revision": revision,
            "gitTree": expected,
            "files": count,
            "contentHashVerified": True,
            "cacheVerified": True,
        }
    with urllib.request.urlopen(
        f"{repository}/+archive/{revision}/{relative}.tar.gz", timeout=120
    ) as response:
        archive_data = response.read(256 * 1024 * 1024 + 1)
    if len(archive_data) > 256 * 1024 * 1024:
        raise ValueError("Rust build source archive exceeds size limit")
    tree = {}
    entries = []
    total = 0
    with tarfile.open(fileobj=io.BytesIO(archive_data), mode="r:gz") as archive:
        for member in archive:
            path = Path(member.name)
            if path.is_absolute() or ".." in path.parts:
                raise ValueError("invalid Rust build source path")
            if member.isdir():
                continue
            if member.isfile():
                total += member.size
                if total > 2 * 1024 * 1024 * 1024:
                    raise ValueError("Rust build sources exceed size limit")
                data = archive.extractfile(member).read()
                mode = "100755" if member.mode & 0o111 else "100644"
            elif member.issym():
                data = member.linkname.encode()
                mode = "120000"
            else:
                raise ValueError("unsupported Rust build source member")
            node = tree
            for part in path.parts[:-1]:
                node = node.setdefault(part, {})
            node[path.name] = (
                mode,
                hashlib.sha1(
                    b"blob " + str(len(data)).encode() + b"\0" + data
                ).digest(),
            )
            entries.append((path, mode, data))

    def tree_hash(node):
        contents = b""
        for name, value in sorted(
            node.items(),
            key=lambda entry: (
                entry[0] + ("/" if isinstance(entry[1], dict) else "")
            ).encode(),
        ):
            mode, digest = (
                ("40000", tree_hash(value)) if isinstance(value, dict) else value
            )
            contents += mode.encode() + b" " + name.encode() + b"\0" + digest
        return hashlib.sha1(
            b"tree " + str(len(contents)).encode() + b"\0" + contents
        ).digest()

    if tree_hash(tree).hex() != expected:
        raise ValueError("Rust source Git tree hash mismatch")
    for path, mode, data in entries:
        destination = directory / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        if mode == "120000":
            target = data.decode()
            if (
                not (destination.parent / target)
                .resolve()
                .is_relative_to(directory.resolve())
            ):
                raise ValueError("Rust source symlink escapes vendor directory")
            if destination.is_symlink() and os.readlink(destination) == target:
                continue
            if destination.exists() or destination.is_symlink():
                raise ValueError(
                    "existing Rust source conflicts with pinned dependency"
                )
            destination.symlink_to(target)
        else:
            if destination.exists():
                if destination.read_bytes() != data:
                    raise ValueError(
                        "existing Rust source conflicts with pinned dependency"
                    )
                continue
            destination.write_bytes(data)
            destination.chmod(0o755 if mode == "100755" else 0o644)
    return {
        "name": "chromium-rust-vendor",
        "repository": repository,
        "revision": revision,
        "gitTree": expected,
        "archiveSha256": hashlib.sha256(archive_data).hexdigest(),
        "files": len(entries),
        "contentHashVerified": True,
    }


def directory_tree(directory):
    contents = b""
    count = 0
    entries = sorted(
        directory.iterdir(),
        key=lambda entry: (
            entry.name + ("/" if entry.is_dir() and not entry.is_symlink() else "")
        ).encode(),
    )
    for entry in entries:
        if entry.is_dir() and not entry.is_symlink():
            mode = "40000"
            digest, children = directory_tree(entry)
            count += children
        else:
            if entry.is_symlink():
                mode, data = "120000", os.readlink(entry).encode()
            elif entry.is_file():
                mode, data = (
                    ("100755" if entry.stat().st_mode & 0o111 else "100644"),
                    entry.read_bytes(),
                )
            else:
                raise ValueError("unsupported Rust source cache member")
            digest = hashlib.sha1(
                b"blob " + str(len(data)).encode() + b"\0" + data
            ).digest()
            count += 1
        contents += mode.encode() + b" " + entry.name.encode() + b"\0" + digest
    return hashlib.sha1(
        b"tree " + str(len(contents)).encode() + b"\0" + contents
    ).digest(), count


def main():
    crate, destination, receipt = map(Path, sys.argv[1:])
    versions = subprocess.check_output(
        [
            sys.executable,
            "-c",
            "import sys;sys.path.insert(0, 'tools');from v8_deps import Var,deps;import json;print(json.dumps([Var('gn_version'),Var('ninja_version'),deps['third_party/icu'],deps['third_party/rust']]))",
        ],
        cwd=crate,
        text=True,
    )
    gn_version, ninja_version, icu_source, rust_source = json.loads(versions)
    receipts = []
    for name, package, version in [
        ("gn", "gn/gn/mac-arm64", gn_version),
        ("ninja", "infra/3pp/tools/ninja/mac-arm64", ninja_version),
    ]:
        print(f"Verifying pinned {name}", flush=True)
        instance_id = metadata(
            "instance/resolve", {"package_name": package, "version": version}
        )["instance_id"]
        instance = metadata(
            "instance", {"package_name": package, "instance_id": instance_id}
        )
        with urllib.request.urlopen(instance["fetch_url"], timeout=120) as response:
            data = response.read()
        digest = hashlib.sha256(data).digest()
        encoded = base64.urlsafe_b64encode(digest + b"\x02").decode().rstrip("=")
        if encoded != instance_id:
            raise ValueError("CIPD archive content hash mismatch")
        directory = destination / name
        directory.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            binary_data = archive.read(name)
        binary = directory / name
        if not binary.exists() or binary.read_bytes() != binary_data:
            with tempfile.NamedTemporaryFile(dir=directory, delete=False) as pending:
                pending.write(binary_data)
                pending_path = Path(pending.name)
            pending_path.chmod(0o755)
            os.replace(pending_path, binary)
        receipts.append(
            {
                "name": name,
                "package": package,
                "version": version,
                "instanceId": instance_id,
                "archiveSha256": digest.hex(),
                "binarySha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "contentHashVerified": True,
            }
        )
    print("Verifying pinned ICU data", flush=True)
    repository, revision = icu_source.rsplit("@", 1)
    url = f"{repository}/+/{revision}/common/"
    with urllib.request.urlopen(url + "?format=JSON", timeout=60) as response:
        listing = json.loads(response.read().decode().split("\n", 1)[1])
    blob = next(entry for entry in listing["entries"] if entry["name"] == "icudtl.dat")
    directory = crate / "third_party/icu/common"
    binary = directory / "icudtl.dat"
    cached = binary.exists()
    if cached:
        data = binary.read_bytes()
    else:
        with urllib.request.urlopen(
            url + "icudtl.dat?format=TEXT", timeout=120
        ) as response:
            encoded_data = response.read(64 * 1024 * 1024 + 1)
        if len(encoded_data) > 64 * 1024 * 1024:
            raise ValueError("ICU data exceeds expected build asset size")
        data = base64.b64decode(encoded_data, validate=True)
    git_hash = hashlib.sha1(
        b"blob " + str(len(data)).encode() + b"\0" + data
    ).hexdigest()
    if git_hash != blob["id"]:
        raise ValueError("ICU Git blob hash mismatch")
    directory.mkdir(parents=True, exist_ok=True)
    if not cached:
        with tempfile.NamedTemporaryFile(dir=directory, delete=False) as pending:
            pending.write(data)
            pending_path = Path(pending.name)
        os.replace(pending_path, directory / "icudtl.dat")
    receipts.append(
        {
            "name": "icudtl.dat",
            "repository": repository,
            "revision": revision,
            "gitBlob": git_hash,
            "sha256": hashlib.sha256(data).hexdigest(),
            "contentHashVerified": True,
            "cacheVerified": cached,
        }
    )
    receipts.append(prepare_rust_sources(crate, rust_source))
    receipt.write_text(json.dumps(receipts, indent=2) + "\n")


if __name__ == "__main__":
    main()
