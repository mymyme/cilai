#!/usr/bin/env python3
"""验证官方 Codex 公共线程协议；业务历史迁移由工作台 Rust 集成测试验证。"""
import argparse
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--harness', required=True, type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    script = Path(__file__).with_name('verify-persistent-thread-harness.mjs')
    completed = subprocess.run(['node', str(script), str(args.harness.resolve(strict=True))], check=True, capture_output=True, text=True)
    result = json.loads(completed.stdout.strip().splitlines()[-1])
    result['historyOwner'] = 'workbench'
    encoded = json.dumps(result, ensure_ascii=False, indent=2) + '\n'
    if args.output:
        args.output.write_text(encoded, encoding='utf-8')
    print(encoded, end='')


if __name__ == '__main__':
    main()
