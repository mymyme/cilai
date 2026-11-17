async function openSavedMemoLink({ service, args, openExternal }) {
  const memo = await service.call('ai_pa_records', { runtimeSessionId: args?.runtimeSessionId, kind: 'memo', request: { action: 'get', id: args?.id } });
  if (typeof memo?.url !== 'string') throw new Error('pa_memo_url_invalid');
  const url = new URL(memo.url);
  if (!['https:', 'http:'].includes(url.protocol) || !url.hostname || url.username || url.password) throw new Error('pa_memo_url_invalid');
  await openExternal(url.href);
  return null;
}
module.exports = { openSavedMemoLink };
