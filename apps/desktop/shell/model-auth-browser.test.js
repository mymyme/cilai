const test = require("node:test");
const assert = require("node:assert/strict");
const { officialAuthorizationUrl } = require("./model-auth-browser");

test("官方授权页可打开，其他来源和协议明确拒绝", () => {
  const url = "https://auth.openai.com/codex/device";
  assert.equal(officialAuthorizationUrl(url), url);
  for (const value of [null, "http://auth.openai.com/codex/device", "https://auth.openai.com.example.test/codex/device", "https://user:secret@auth.openai.com/codex/device", "https://auth.openai.com:8443/codex/device", "file:///tmp/auth", "https://auth.openai.com/oauth/authorize", "https://auth.openai.com/codex/device?redirect=synthetic-test"]) {
    assert.throws(() => officialAuthorizationUrl(value), /model_auth_url_invalid/);
  }
});
