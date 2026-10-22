import { expect, it } from "vitest";
import { skillMessageText } from "./skill-message";
it("从原生正文派生技能交付显示，不需要单独保存正文副本", () => {
  const body = '完成。\n```cilai-skill\n{"name":"分析","description":"说明","instructions":"执行","files":[]}\n```\n后续说明';
  expect(skillMessageText(body)).toBe("完成。\n后续说明");
  expect(skillMessageText(body.replace("\n```\n", "\n"))).toContain("cilai-skill");
  expect(skillMessageText(body + body)).toBe(body + body);
});
