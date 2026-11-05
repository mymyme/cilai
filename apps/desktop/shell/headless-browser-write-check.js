const { withBrowserFixture } = require("./headless-browser-fixture");

async function checkBrowserWriteApproval({ runtime, contents, approvals, post }) {
  const previousNotify = approvals.notify;
  let seen = null;
  return withBrowserFixture(runtime, contents, async (page, tabId) => {
    try {
      const protocol = runtime.sessionOf().protocol;
      protocol.handle("https", () => new Response(
        '<body style="margin:0"><button style="width:80px;height:60px" onclick="window.__cilaiBrowserCheckClicks++">自检</button><script>window.__cilaiBrowserCheckClicks=0</script>',
        { headers: { "content-type": "text/html; charset=utf-8" } },
      ));
      const fixtureUrl = "https://cilai-browser-approval-check.invalid/";
      try {
        await runtime.navigate(fixtureUrl, null);
      } finally {
        protocol.unhandle("https");
      }
      if (runtime.current(null)?.contents !== page || page.getURL() !== fixtureUrl) {
        throw new Error(`浏览器审批自检页面尚未准备好：${JSON.stringify(page.getURL())}`);
      }
      const snapshot = await runtime.snapshot(200, tabId, null);
      const ref = snapshot.nodes.find(node => node.role === "button" && node.name === "自检")?.ref;
      if (!ref) throw new Error("浏览器审批自检未读取到按钮句柄");
      approvals.notify = (event, payload) => {
        if (event === "browser-approval-request") {
          seen = payload;
          approvals.resolve(payload.confirmationId, false);
        } else {
          previousNotify(event, payload);
        }
      };
      const reply = await post({
        jsonrpc: "2.0", id: 6, method: "tools/call",
        params: { name: "browser_click", arguments: { ref, purpose: "自检" } },
      }).then((response) => response.json());
      const clickCount = await page.executeJavaScript("window.__cilaiBrowserCheckClicks");
      const stale = await runtime.captureWriteTarget(null, { ref });
      runtime.setDriver("employee");
      const paused = await runtime.click(stale).then(() => false, error => error.message.includes("写操作已暂停"));
      runtime.setDriver("ai");
      const staleRejected = await runtime.click(stale).then(() => false, error => error.message.includes("目标已变化"));
      const freshTarget = await runtime.captureWriteTarget(null, { ref });
      const availableAfterReturn = runtime.assertWriteTarget(freshTarget).contents === page;
      const afterTakeover = await page.executeJavaScript("window.__cilaiBrowserCheckClicks");
      return {
        paused, staleRejected, availableAfterReturn, afterTakeover,
        raised: seen !== null,
        rejected: reply.result?.isError === true && reply.result.content.some(
          (item) => item.type === "text" && item.text.includes("员工拒绝了本次操作"),
        ),
        clickCount,
        reason: reply.result?.content?.filter((item) => item.type === "text").map((item) => item.text).join("; ")
          ?? reply.error?.message ?? "工具响应没有文本结果",
      };
    } finally {
      if (seen) approvals.resolve(seen.confirmationId, false);
      approvals.notify = previousNotify;
    }
  });
}

module.exports = { checkBrowserWriteApproval };
