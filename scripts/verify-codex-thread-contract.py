#!/usr/bin/env python3
"""验证官方 Codex 公共线程协议；业务历史迁移由工作台 Rust 集成测试验证。"""
import argparse
import json
from pathlib import Path
import subprocess


def parse_harness_report(stdout):
    reports = [json.loads(line) for line in stdout.splitlines()
               if line.startswith('{"persistentResume":')]
    if len(reports) != 1 or reports[0].get('persistentResume') is not True:
        raise ValueError('必须读取唯一通过的公共线程协议报告')
    return reports[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--harness', required=True, type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    script = Path(__file__).with_name('verify-persistent-thread-harness.mjs')
    completed = subprocess.run(['node', str(script), str(args.harness.resolve(strict=True))], check=True, capture_output=True, text=True)
    result = parse_harness_report(completed.stdout)
    result['historyOwner'] = 'workbench'
    encoded = json.dumps(result, ensure_ascii=False, indent=2) + '\n'
    if args.output:
        args.output.write_text(encoded, encoding='utf-8')
    print(encoded, end='')


if __name__ == '__main__':
    main()
