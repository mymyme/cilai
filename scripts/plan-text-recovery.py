"""Build an explicit recovery plan from canonical exports and original JSONL evidence."""
import argparse
import hashlib
import json
import re
from collections import Counter
from pathlib import Path


def strings(value, pointer="", depth=0, decode=True):
    if isinstance(value, dict):
        for key, item in value.items():
            if re.search(r"password|secret|api.?key|authorization|base64", key, re.I):
                continue
            escaped = key.replace("~", "~0").replace("/", "~1")
            yield from strings(item, pointer + "/" + escaped, depth, decode)
    elif isinstance(value, list):
        for index, item in enumerate(value):
            yield from strings(item, pointer + "/" + str(index), depth, decode)
    elif isinstance(value, str):
        yield pointer, value
        if decode and depth < 3 and value.lstrip().startswith(("{", "[")):
            try:
                nested = json.loads(value)
            except (ValueError, RecursionError):
                return
            yield from strings(nested, pointer, depth + 1)


def matcher(before):
    return re.compile("".join("([^\ufffd])" if part.startswith("\ufffd") else re.escape(part)
                              for part in re.split(r"(\ufffd+)", before) if part))


def plan(export, evidence_root):
    damaged = [(pointer, value) for pointer, value in strings(export["state"], decode=False)
               if "\ufffd" in value and pointer.split("/")[1] in ("runs", "messages", "conversations")]
    patterns = [matcher(value.rstrip("…")) for _, value in damaged]
    anchors = {}
    for index, (_, value) in enumerate(damaged):
        intact = re.split("\ufffd+", value.rstrip("…"))
        anchor = max(intact, key=len)[:64]
        if len(anchor) >= 2:
            anchors.setdefault(anchor, []).append(index)
    search = re.compile("|".join(re.escape(value) for value in sorted(anchors, key=len, reverse=True)))
    overlaps = {anchor: {index for other, indices in anchors.items() if other in anchor for index in indices}
                for anchor in anchors}
    candidates = [dict() for _ in damaged]
    files = sorted((evidence_root / "runs").glob("*.harness.jsonl"))
    files += sorted((evidence_root / "cilai-harness/sessions").rglob("*.jsonl"))
    digests = {}
    for file in files:
        with file.open(encoding="utf-8", errors="strict") as stream:
            for line_number, line in enumerate(stream, 1):
                try:
                    value = json.loads(line)
                except ValueError:
                    continue
                for _, original in strings(value):
                    if not original or len(original) > 2_000_000:
                        continue
                    for text in {original, " ".join(original.split())}:
                        indices = {index for hit in search.finditer(text) for index in overlaps[hit.group()]}
                        for index in indices:
                            before = damaged[index][1]
                            for hit in patterns[index].finditer(text):
                                after = hit.group() + ("…" if before.endswith("…") else "")
                                if any(len(char.encode("utf-8")) < 2 for char in hit.groups()):
                                    continue
                                if after not in candidates[index]:
                                    candidates[index][after] = (file, line_number)
        print("evidence", file.name, flush=True)
    patches = []
    unresolved = []
    for (pointer, before), found in zip(damaged, candidates):
        if len(found) != 1:
            unresolved.append({"pointer": pointer, "candidates": len(found)})
            continue
        after, (file, line) = next(iter(found.items()))
        if file not in digests:
            digests[file] = hashlib.sha256(file.read_bytes()).hexdigest()
        patches.append({"pointer": pointer, "before": before, "after": after,
                        "evidence": {"file": str(file), "line": line, "digest": digests[file]}})
    return {"stateDigest": export["stateDigest"], "ledgerDigest": export["ledgerDigest"],
            "patches": patches, "unresolved": unresolved,
            "summary": {"damagedFields": len(damaged), "restorableFields": len(patches),
                        "unresolvedFields": len(unresolved),
                        "collections": dict(Counter(p["pointer"].split("/")[1] for p in patches))}}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("export", type=Path)
    parser.add_argument("evidence_root", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    result = plan(json.loads(args.export.read_text()), args.evidence_root)
    args.output.write_text(json.dumps(result, ensure_ascii=False))
    print(json.dumps(result["summary"], ensure_ascii=False))
