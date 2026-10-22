function officialAuthorizationUrl(value) {
  if (typeof value !== "string") throw new Error("model_auth_url_invalid");
  let url;
  try { url = new URL(value); } catch { throw new Error("model_auth_url_invalid"); }
  if (url.protocol !== "https:" || url.hostname !== "auth.openai.com" || url.port || url.username || url.password || url.hash || url.search || url.pathname !== "/codex/device") {
    throw new Error("model_auth_url_invalid");
  }
  return url.href;
}

module.exports = { officialAuthorizationUrl };
