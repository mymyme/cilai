import "./startup-screen.css";

const slices = [
  { position: "upper-left", d: "M10 22.5C16.3 19.8 22.6 20 28 23.5V29C21.5 28.7 15.5 27.4 10 24.8V22.5Z", fill: "startup-left" },
  { position: "upper-right", d: "M54 22.5C47.7 19.8 41.4 20 36 23.5V29C42.5 28.7 48.5 27.4 54 24.8V22.5Z", fill: "startup-right" },
  { position: "lower-left", d: "M28 27.5C25.5 35.8 19.3 43.1 9 49.5V37.6C16.7 34.8 22.7 31.4 28 27.5Z", fill: "startup-left" },
  { position: "lower-right", d: "M36 27.5C38.5 35.8 44.7 43.1 55 49.5V37.6C47.3 34.8 41.3 31.4 36 27.5Z", fill: "startup-right" },
];

/** 启动等待页：品牌标志的语义核心先亮起，四段切片依次聚合成完整标志，循环表示等待。 */
function AnimatedSymbol() {
  return <svg className="startup-symbol" viewBox="0 0 64 56" aria-hidden="true">
    <defs>
      <linearGradient id="startup-core" x1="27" y1="7" x2="37" y2="17" gradientUnits="userSpaceOnUse"><stop stopColor="#79F0B9"/><stop offset="1" stopColor="#66D9F1"/></linearGradient>
      <linearGradient id="startup-left" x1="9" y1="22" x2="30" y2="48" gradientUnits="userSpaceOnUse"><stop stopColor="#66D9F1"/><stop offset="1" stopColor="#3CA9EF"/></linearGradient>
      <linearGradient id="startup-right" x1="55" y1="22" x2="34" y2="48" gradientUnits="userSpaceOnUse"><stop stopColor="#8B86FF"/><stop offset="1" stopColor="#5374E9"/></linearGradient>
    </defs>
    <rect className="startup-core" x="27" y="7" width="10" height="10" rx="1.5" fill="url(#startup-core)"/>
    {slices.map(({ position, d, fill }) => <path key={position} className="startup-slice" data-position={position} d={d} fill={`url(#${fill})`}/>)}
  </svg>;
}

export function StartupScreen({ error, onReload, label = "正在启动" }: { error?: string | null; onReload?: () => void; label?: string }) {
  return <section className="startup-screen" data-state={error ? "error" : "loading"} aria-busy={!error}>
    <AnimatedSymbol/>
    {error
      ? <div className="startup-error" role="alert"><h2>宿主状态无法核实</h2><p>{error}</p>{onReload ? <button type="button" onClick={onReload}>重新打开</button> : null}</div>
      : <h2 className="startup-label" role="status">{label}</h2>}
  </section>;
}
