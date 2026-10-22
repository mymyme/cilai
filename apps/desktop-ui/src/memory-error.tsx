type MemoryFailure = { summary: string; fields: [string, string][] };

function memoryFailure(error: string): MemoryFailure | null {
  const start = error.indexOf("{");
  if (start < 0) return null;
  try {
    const payload = JSON.parse(error.slice(start));
    const failure = payload?.error;
    if (!failure || typeof failure !== "object" || typeof failure.message !== "string") return null;
    const prefix = error.slice(0, start).trim();
    const title = typeof failure.title === "string" && failure.title.trim() ? failure.title : "记忆处理失败";
    const diagnostics = failure.diagnostics;
    const fields: [string, unknown][] = [
      ["原因", failure.message],
      ["错误码", failure.code],
      ["失败阶段", diagnostics?.stage],
      ["参数", diagnostics?.parameter],
      ["上游状态", diagnostics?.upstream_status],
      ["请求编号", payload.request_id],
      ["上游请求", diagnostics?.upstream_request_id],
    ];
    return {
      summary: prefix + title,
      fields: fields.flatMap(([label, value]) =>
        (typeof value === "string" && value.trim()) || (typeof value === "number" && Number.isFinite(value))
          ? [[label, String(value)] as [string, string]] : []),
    };
  } catch {
    return null;
  }
}

export function MemoryErrorNotice({ error }: { error: string }) {
  const failure = memoryFailure(error);
  if (!failure) return <p className="settings-error">{error}</p>;
  return <div className="settings-memory-error">
    <p className="settings-error">{failure.summary}</p>
    <details key={error} className="settings-error-details">
      <summary>详情</summary>
      <dl>{failure.fields.map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl>
    </details>
  </div>;
}
