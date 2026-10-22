import { invoke } from "./host";
import { listen } from "./host";
import { useEffect, useState } from "react";
import { createPortal } from "./page-portal";
import "./browser-approval.css";

/**
 * AI 浏览器写操作的确认。
 *
 * 点击与输入会在外部站点留下真实痕迹——提交表单、下单、发消息都由它们构成，因此逐次
 * 交由员工确认。确认卡片必须说清在哪个站点、做什么、用什么参数：只说"AI 想点击"而不
 * 说在哪、点什么，员工无从判断。
 *
 * 可以按站点授权，但只对点击这类操作开放，且只在本次对话内有效。一个流程往往由十几次
 * 点击构成，逐次弹窗会让员工在疲劳中一路点"允许"，那样的审批只是形式。涉及写入文本的
 * 操作（输入、设值）不给这个选项，每次都要单独看过——授权"在这个站点点击"不等于授权
 * "在这个站点输入任何内容"。
 */

type ApprovalRequest = {
  confirmationId: string;
  tool: string;
  site: string;
  summary: string;
  arguments: Record<string, unknown>;
  riskLevel: string;
  /** 宿主说了算：这次能不能给"本站点都允许"。界面不自行判断。 */
  allowSiteGrant?: boolean;
};

const TOOL_LABELS: Record<string, string> = {
  browser_click: "点击页面",
  browser_type: "输入文本",
  browser_set_value: "填写表单",
};

/** 参数名用员工读得懂的说法呈现；未知参数按原名显示，不隐藏。 */
const ARGUMENT_LABELS: Record<string, string> = {
  ref: "目标元素",
  x: "横坐标",
  y: "纵坐标",
  text: "输入内容",
  value: "填入的值",
  fieldHint: "目标字段",
  purpose: "操作目的",
};

export function BrowserApprovalSurface() {
  const [queue, setQueue] = useState<ApprovalRequest[]>([]);

  useEffect(() => {
    let disposed = false;
    const disposers: Array<() => void> = [];
    const track = (pending: Promise<() => void>) => {
      void pending.then((dispose) => {
        if (disposed) dispose();
        else disposers.push(dispose);
      });
    };
    track(listen<ApprovalRequest>("browser-approval-request", (request) => {
      setQueue((current) => [...current, request]);
    }));
    // 超时或别处已决定时把卡片撤掉，避免员工对着一张已经无效的卡片做决定
    track(listen<{ confirmationId: string }>("browser-approval-closed", (closed) => {
      setQueue((current) =>
        current.filter((item) => item.confirmationId !== closed.confirmationId));
    }));
    return () => {
      disposed = true;
      for (const dispose of disposers) dispose();
    };
  }, []);

  const current = queue[0];
  if (!current) return null;

  const decide = (approved: boolean, grantSite = false) => {
    // 先移除再回传：回传失败也不该让卡片卡在界面上，宿主那边超时会按拒绝处理
    setQueue((rest) => rest.filter((item) => item.confirmationId !== current.confirmationId));
    void invoke("browser_approval_resolve", {
      confirmationId: current.confirmationId,
      approved,
      grantSite,
    }).catch(() => undefined);
  };

  return createPortal(
    <div
      aria-label="确认浏览器操作"
      aria-modal="true"
      className="confirm-backdrop"
      role="alertdialog"
    >
      <section className="confirm-dialog browser-approval">
        <h2>确认 AI 的浏览器操作</h2>
        <div className="confirm-body">
          <p className="browser-approval__summary">{current.summary}</p>
          <dl className="browser-approval__facts">
            <dt>站点</dt>
            <dd>{current.site}</dd>
            <dt>操作</dt>
            <dd>{TOOL_LABELS[current.tool] ?? current.tool}</dd>
            {Object.entries(current.arguments)
              .filter(([, value]) => value !== undefined && value !== null && value !== "")
              .map(([key, value]) => (
                <div className="browser-approval__row" key={key}>
                  <dt>{ARGUMENT_LABELS[key] ?? key}</dt>
                  <dd>{String(value)}</dd>
                </div>
              ))}
          </dl>
          {queue.length > 1 ? (
            <p className="browser-approval__queue">另有 {queue.length - 1} 项待确认</p>
          ) : null}
        </div>
        <div className="confirm-actions">
          <button onClick={() => decide(false)} type="button">
            拒绝
          </button>
          {current.allowSiteGrant ? (
            <button onClick={() => decide(true, true)} type="button">
              本站点本次对话都允许
            </button>
          ) : null}
          <button className="danger" onClick={() => decide(true)} type="button">
            批准本次
          </button>
        </div>
      </section>
    </div>,
    document.body,
  );
}
