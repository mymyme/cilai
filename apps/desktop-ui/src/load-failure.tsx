import { describeServerError } from "./server-error";

/**
 * "读不出来"和"这里是空的"必须长得不一样。
 *
 * 此前所有页面读取失败后都只是 setLoading(false)，于是落进各自的空状态：文件页劝你
 * 新建工作区、项目页说"没有可访问的项目"、搜索页说"没有匹配结果"。你的数据好端端在，
 * 界面却在告诉你它不存在——最糟的是项目页那句，它还顺带给了个错误的解释。
 *
 * 空状态是一句关于事实的陈述，只有真的读成功且为空时才能说。
 */
export function LoadFailure(props: { what: string; error: unknown; onRetry: () => void }) {
  return <div className="load-failure" role="alert">
    <strong>{props.what}读取失败</strong>
    <p>{describeServerError(props.error)}</p>
    <button className="secondary-button" onClick={props.onRetry} type="button">重试</button>
  </div>;
}
