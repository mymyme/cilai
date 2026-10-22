const assert = require("node:assert/strict");
const { afterEach, test } = require("node:test");
const { BrowserApprovals } = require("./browser-approval");

const activeApprovals = new Set();

afterEach(() => {
  for (const approvals of activeApprovals) approvals.rejectAll("测试结束");
  activeApprovals.clear();
});

/** 收集发给界面的事件，并能取出最近一次审批请求。 */
function harness() {
  const events = [];
  const approvals = new BrowserApprovals((name, payload) => events.push({ name, payload }));
  activeApprovals.add(approvals);
  return {
    approvals,
    events,
    last: () => events.filter((item) => item.name === "browser-approval-request").at(-1)?.payload,
  };
}

const clickOn = (site, scope = "conv-1") => ({
  tool: "browser_click", site, scope, summary: `在 ${site} 点击`, arguments: {},
});

/*
 * 粒度按站点而不是逐次：一个流程往往由十几次点击构成，逐次弹窗会让员工在疲劳中一路
 * 点"允许"，那样的审批只是形式。但授权的边界必须守死——换站点、换对话、涉及写入文本
 * 都要重新问。
 */
test("授权之后同站同对话的点击不再打扰员工", async () => {
  const { approvals, last } = harness();
  const first = approvals.request(clickOn("https://a.example"));
  approvals.resolve(last().confirmationId, true, /*grantSite*/ true);
  assert.equal((await first).approved, true);

  const second = await approvals.request(clickOn("https://a.example"));
  assert.equal(second.approved, true);
  assert.equal(second.viaGrant, true, "应当是凭授权放行，而不是又问了一次");
});

test("换一个站点仍然要问", async () => {
  const { approvals, last } = harness();
  const first = approvals.request(clickOn("https://a.example"));
  approvals.resolve(last().confirmationId, true, true);
  await first;
  let settled = false;
  void approvals.request(clickOn("https://b.example")).then(() => { settled = true; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settled, false, "别的站点不该被上一个站点的授权覆盖");
  assert.equal(last().site, "https://b.example");
});

test("换一个对话仍然要问", async () => {
  const { approvals, last } = harness();
  const first = approvals.request(clickOn("https://a.example", "conv-1"));
  approvals.resolve(last().confirmationId, true, true);
  await first;
  let settled = false;
  void approvals.request(clickOn("https://a.example", "conv-2")).then(() => { settled = true; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settled, false, "授权只在给出它的那次对话内成立");
});

test("涉及写入文本的操作永远单问，站点授权盖不住", async () => {
  const { approvals, last } = harness();
  const first = approvals.request(clickOn("https://a.example"));
  approvals.resolve(last().confirmationId, true, true);
  await first;

  let settled = false;
  void approvals.request({
    tool: "browser_type", site: "https://a.example", scope: "conv-1",
    alwaysAsk: true, summary: "输入", arguments: {},
  }).then(() => { settled = true; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settled, false, "输入类操作的风险不随站点信任而降低");
  assert.equal(last().allowSiteGrant, false, "也不该给它'本站点都允许'的选项");
});

test("站点取不到时不给授权选项，授权范围必须是员工看得见的那个站点", async () => {
  const { approvals, last } = harness();
  void approvals.request(clickOn("未打开页面"));
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(last().allowSiteGrant, false);
});

test("界面即便传了授权标志，宿主不允许授权的请求也建立不了授权", async () => {
  const { approvals, last } = harness();
  const first = approvals.request({
    tool: "browser_type", site: "https://a.example", scope: "conv-1",
    alwaysAsk: true, summary: "输入", arguments: {},
  });
  const outcome = approvals.resolve(last().confirmationId, true, /*grantSite*/ true);
  assert.equal(outcome.grantedSite, null, "能不能授权由宿主判定，不由界面传参决定");
  await first;
  let settled = false;
  void approvals.request({
    tool: "browser_type", site: "https://a.example", scope: "conv-1",
    alwaysAsk: true, summary: "再输入", arguments: {},
  }).then(() => { settled = true; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settled, false);
});

test("员工接手后收回该对话的授权", async () => {
  const { approvals, last } = harness();
  const first = approvals.request(clickOn("https://a.example"));
  approvals.resolve(last().confirmationId, true, true);
  await first;
  approvals.revokeScope("conv-1");
  let settled = false;
  void approvals.request(clickOn("https://a.example")).then(() => { settled = true; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settled, false, "员工动过页面之后，原来的授权不该继续生效");
});

test("拒绝时不建立授权", async () => {
  const { approvals, last } = harness();
  const first = approvals.request(clickOn("https://a.example"));
  approvals.resolve(last().confirmationId, false, true);
  assert.equal((await first).approved, false);
  let settled = false;
  void approvals.request(clickOn("https://a.example")).then(() => { settled = true; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settled, false);
});
