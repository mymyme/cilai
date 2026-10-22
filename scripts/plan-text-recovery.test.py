import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("recovery", Path(__file__).with_name("plan-text-recovery.py"))
recovery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recovery)


class RecoveryPlanTests(unittest.TestCase):
    def build(self, damaged, originals):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "runs").mkdir()
            (root / "runs/original.harness.jsonl").write_text(
                "\n".join(json.dumps({"receipt": text}, ensure_ascii=False) for text in originals),
                encoding="utf-8")
            return recovery.plan({"stateDigest": "state", "ledgerDigest": "ledger",
                                  "state": {"runs": [{"output": text} for text in damaged]}}, root)

    def test_unique_original_nested_json_and_truncated_summary(self):
        result = self.build(["测试��能/原图.png", "这是完整中��原文…"],
                            ['{"path":"测试技能/原图.png"}', "这是完整中文原文，继续说明"])
        self.assertEqual([p["after"] for p in result["patches"]],
                         ["测试技能/原图.png", "这是完整中文原文…"])
        self.assertEqual(result["unresolved"], [])
        self.assertEqual(result["patches"][0]["evidence"]["line"], 1)

    def test_missing_or_ambiguous_source_is_unchanged(self):
        result = self.build(["测试��能/原图.png", "缺少原始��据"],
                            ["测试技能/原图.png", "测试功能/原图.png"])
        self.assertEqual(result["patches"], [])
        self.assertEqual([p["candidates"] for p in result["unresolved"]], [2, 0])

    def test_does_not_guess_ascii_or_rewrite_intact_characters(self):
        result = self.build(["测试��能/原图.png"], ["测试a能/原图.png", "其他技能/原图.png"])
        self.assertEqual(result["patches"], [])

    def test_overlapping_anchors_and_normalized_activity_are_both_restored(self):
        result = self.build(["测试��能", "测试��能/原图.png", "调用工具  生成中��图片…"],
                            ["测试技能/原图.png", "调用工具  生成中文图片，完整回执"])
        self.assertEqual(len(result["patches"]), 3)

    def test_intact_source_excerpt_survives_unrelated_damage_in_the_same_receipt(self):
        result = self.build(["测试��能/原图.png"], ["其他字段��；测试技能/原图.png"])
        self.assertEqual(result["patches"][0]["after"], "测试技能/原图.png")


if __name__ == "__main__":
    unittest.main()
