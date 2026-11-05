import { invoke } from "./host";
import { useCallback, useEffect, useRef, useState } from "react";
import { rpc } from "./app-server-rpc";
import { usePageActive } from "./page-visibility";
import type {
  ComputeTextDiffParams,
  ComputeTextDiffResult,
  DiffSourceRef,
  OpenPreviewParams,
  OpenPreviewResult,
  PreviewSource,
  ReadTextDiffParams,
  ReadTextDiffResult,
  TextDiffRow,
} from "./generated/protocol";
import { auditMoment } from "./datetime";
import { ERROR_LABELS, SOURCE_LABELS } from "./preview-labels";

const DIFF_ERROR_LABELS: Record<string, string> = {
  ...ERROR_LABELS,
  text_diff_source_too_large: "任一来源超过 4 MiB，不能生成逐行差异。",
  text_diff_too_many_rows: "差异行数超过当前上限，建议使用外部工具比较。",
  text_diff_line_too_large: "文件包含过长单行，不能安全生成内置差异。",
  text_diff_ticket_not_found: "差异结果已失效，请重新生成。",
  text_diff_ticket_expired: "差异结果已过期，请重新生成。",
  text_diff_source_changed: "比较来源已经变化，当前差异已失效。",
  text_diff_read_failed: "来源存在，但差异读取失败。",
};

type DiffState =
  | { phase: "loading" }
  | { phase: "unsupported"; source: PreviewSource; reason: string }
  | { phase: "unavailable"; source: PreviewSource; reason: string }
  | {
      phase: "ready";
      result: ComputeTextDiffResult;
      rows: TextDiffRow[];
      nextCursor: number | null;
      loadingMore: boolean;
    }
  | { phase: "error"; reason: string };

export interface FileDiffPaneProps {
  workspaceId: string;
  localFileId: string;
  leftSource: PreviewSource;
  rightSource: PreviewSource;
  onReadyChange?: (ready: boolean) => void;
  serviceScope?: string | null;
  fileRevision?: string | null;
}

export function FileDiffPane(props: FileDiffPaneProps) {
  const key = JSON.stringify([props.serviceScope, props.workspaceId, props.localFileId, props.leftSource, props.rightSource, props.fileRevision]);
  return <ActiveFileDiffPane key={key} {...props}/>;
}

function ActiveFileDiffPane({
  workspaceId,
  localFileId,
  leftSource,
  rightSource,
  onReadyChange,
}: FileDiffPaneProps) {
  const { state, loadMore, retry } = useTextDiff({ workspaceId, localFileId, leftSource, rightSource });
  useEffect(() => {
    onReadyChange?.(state.phase === "ready");
  }, [onReadyChange, state.phase]);
  return (
    <section className="diff-pane" aria-label="文本差异">
      <header className="diff-source-header">
        <SourceHeader side="旧文档" source={leftSource} />
        <SourceHeader side="新文档" source={rightSource} />
      </header>
      <DiffBody loadMore={loadMore} retry={retry} state={state} />
    </section>
  );
}

function SourceHeader({ side, source }: { side: string; source: PreviewSource }) {
  return (
    <div>
      <small>{side}</small>
      <strong>{SOURCE_LABELS[source]}</strong>
    </div>
  );
}

function DiffBody({ state, loadMore, retry }: { state: DiffState; loadMore: () => void; retry: () => void }) {
  if (state.phase === "loading") {
    return <DiffMessage title="正在读取两个来源并生成差异" />;
  }
  if (state.phase === "unavailable") {
    return (
      <DiffMessage
        detail="该来源正文尚未物化，未使用其他版本替代。"
        source={state.source}
        title="比较来源不可用"
      />
    );
  }
  if (state.phase === "unsupported") {
    return (
      <DiffMessage
        detail="该文件格式不支持文本差异，未将二进制或未知格式伪装成文本。"
        source={state.source}
        title="格式不支持文本比较"
      />
    );
  }
  if (state.phase === "error") {
    return (
      <DiffMessage
        detail={DIFF_ERROR_LABELS[state.reason] ?? "差异生成失败，请查看诊断后重新生成。"}
        title={state.reason === "text_diff_source_changed" ? "差异已失效" : "差异生成失败"}
        retry={retry}
      />
    );
  }
  const { result, rows } = state;
  return (
    <div>
      <div className="diff-summary" aria-label="变更摘要">
        <span className="insert">新增 {result.summary.addedLines} 行</span>
        <span className="delete">删除 {result.summary.deletedLines} 行</span>
        <span>未变 {result.summary.unchangedLines} 行</span>
        <span>{result.summary.changeBlocks} 个变更块</span>
        <span>共 {result.totalRows} 行</span>
      </div>
      <div className="diff-revisions">
        <span>{sourceRevision(result.left)}</span>
        <span>{sourceRevision(result.right)}</span>
      </div>
      <div className="diff-rows" role="table" aria-label="新旧文档并排差异">
        {rows.map((row, index) => (
          <div className={`diff-row ${row.kind}`} key={`${index}-${row.leftLine}-${row.rightLine}`} role="row">
            <div className={`diff-cell old ${row.kind === "delete" ? "changed" : ""}`}>
              <span className="diff-line-number">{row.leftLine ?? ""}</span>
              <span className="diff-marker">{row.kind === "delete" ? "−" : " "}</span>
              <code>{row.kind === "insert" ? "" : row.text}{row.kind !== "insert" && !row.hasLineTerminator ? " [无行终止符]" : ""}</code>
            </div>
            <div className={`diff-cell current ${row.kind === "insert" ? "changed" : ""}`}>
              <span className="diff-line-number">{row.rightLine ?? ""}</span>
              <span className="diff-marker">{row.kind === "insert" ? "+" : " "}</span>
              <code>{row.kind === "delete" ? "" : row.text}{row.kind !== "delete" && !row.hasLineTerminator ? " [无行终止符]" : ""}</code>
            </div>
          </div>
        ))}
      </div>
      {state.nextCursor !== null ? (
        <button
          className="diff-load-more"
          disabled={state.loadingMore}
          onClick={loadMore}
          type="button"
        >
          {state.loadingMore
            ? "正在验证来源并读取下一页"
            : `加载更多（已显示 ${rows.length}/${result.totalRows} 行）`}
        </button>
      ) : null}
    </div>
  );
}

function DiffMessage({
  title,
  detail,
  source,
  retry,
}: {
  title: string;
  detail?: string;
  source?: PreviewSource;
  retry?: () => void;
}) {
  return (
    <div className="preview-message" role="status">
      {source ? <span className="preview-source-label">{SOURCE_LABELS[source]}</span> : null}
      <h3>{title}</h3>
      {detail ? <p>{detail}</p> : null}
      {retry ? <button type="button" onClick={retry}>重试比较</button> : null}
    </div>
  );
}

type DiffRequest = { workspaceId: string; localFileId: string; leftSource: PreviewSource; rightSource: PreviewSource };

class DiffResourceOwner {
  readonly controller = new AbortController();
  private readonly resources = new Map<string, string>();
  accept(method: string, ticket: string | null) {
    if (!ticket) return;
    if (this.controller.signal.aborted) {
      this.release(method, ticket);
      this.controller.signal.throwIfAborted();
    }
    this.resources.set(ticket, method);
  }
  private release(method: string, ticketId: string) {
    void rpc(method, { ticketId }).catch(error => console.error("差异资源未能释放", error));
  }
  close() {
    this.controller.abort();
    for (const [ticket, method] of this.resources) this.release(method, ticket);
    this.resources.clear();
  }
}

function useTextDiff(params: DiffRequest) {
  const active = usePageActive();
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<DiffState>({ phase: "loading" });
  const owned = useRef<DiffResourceOwner | null>(null);
  useEffect(() => {
    if (!active) return;
    const owner = new DiffResourceOwner();
    owned.current = owner;
    setState({ phase: "loading" });
    void loadTextDiff(params, owner).then(next => {
      if (!owner.controller.signal.aborted) {
        if (next.phase !== "ready") owner.close();
        setState(next);
      }
    }).catch((error: unknown) => {
      if (!owner.controller.signal.aborted) {
        owner.close();
        setState({ phase: "error", reason: extractDiffReason(error) });
      }
    });
    return () => { owner.close(); if (owned.current === owner) owned.current = null; };
  }, [active, attempt, params.leftSource, params.localFileId, params.rightSource, params.workspaceId]);
  const loadMore = useCallback(() => {
    const owner = owned.current;
    if (!owner || owner.controller.signal.aborted || state.phase !== "ready" || state.nextCursor === null || state.loadingMore) return;
    const { result, nextCursor } = state;
    setState({ ...state, loadingMore: true });
    void readDiffPage(result.diffTicketId, nextCursor, owner.controller.signal)
      .then((page) => {
        setState((current) => {
          if (
            current.phase !== "ready" ||
            current.result.diffTicketId !== result.diffTicketId ||
            current.nextCursor !== nextCursor
          ) {
            return current;
          }
          return {
            ...current,
            rows: [...current.rows, ...page.rows],
            nextCursor: page.nextCursor,
            loadingMore: false,
          };
        });
      })
      .catch((error: unknown) => {
        if (owner.controller.signal.aborted) return;
        owner.close();
        setState((current) =>
          current.phase === "ready" && current.result.diffTicketId === result.diffTicketId
            ? { phase: "error", reason: extractDiffReason(error) }
            : current,
        );
      });
  }, [state]);
  return { state, loadMore, retry: () => setAttempt(value => value + 1) };
}

async function loadTextDiff(params: DiffRequest, owner: DiffResourceOwner): Promise<DiffState> {
  const signal = owner.controller.signal;
  const open = async (source: PreviewSource) => {
    const descriptor = await openSource(params.workspaceId, params.localFileId, source);
    owner.accept("textResource/release", descriptor.ticketId);
    signal.throwIfAborted();
    return descriptor;
  };
  const [left, right] = await Promise.all([open(params.leftSource), open(params.rightSource)]);
  if (left.availability !== "ready") {
    return unavailableState(left);
  }
  if (right.availability !== "ready") {
    return unavailableState(right);
  }
  const computeParams: ComputeTextDiffParams = {
    left: diffSource(left),
    right: diffSource(right),
  };
  const result = await invoke<ComputeTextDiffResult>("text_diff_compute", { params: computeParams });
  owner.accept("textDiff/release", result.diffTicketId);
  const page = await readDiffPage(result.diffTicketId, 0, signal);
  return {
    phase: "ready",
    result,
    rows: page.rows,
    nextCursor: page.nextCursor,
    loadingMore: false,
  };
}

async function readDiffPage(diffTicketId: string, cursor: number, signal: AbortSignal): Promise<ReadTextDiffResult> {
  const params: ReadTextDiffParams = {
    diffTicketId,
    cursor,
    maxRows: 200,
    maxBytes: 256 * 1024,
  };
  signal.throwIfAborted();
  const page = await invoke<ReadTextDiffResult>("text_diff_read", { params });
  signal.throwIfAborted();
  if (page.cursor !== cursor) throw new Error("text_diff_read_failed");
  if (page.nextCursor !== null && page.nextCursor <= cursor) {
    throw new Error("text_diff_read_failed");
  }
  return page;
}

function openSource(
  workspaceId: string,
  localFileId: string,
  source: PreviewSource,
): Promise<OpenPreviewResult> {
  const params: OpenPreviewParams = { workspaceId, localFileId, source };
  return invoke<OpenPreviewResult>("preview_open", { params });
}

function diffSource(descriptor: OpenPreviewResult): DiffSourceRef {
  if (!descriptor.ticketId || !descriptor.contentRevision) {
    throw new Error("text_diff_invalid_request");
  }
  return {
    ticketId: descriptor.ticketId,
    source: descriptor.source,
    contentRevision: descriptor.contentRevision,
    label: descriptor.relativePath,
    version: null,
    modifiedAt: descriptor.sourceModifiedAt,
  };
}

function unavailableState(descriptor: OpenPreviewResult): DiffState {
  return {
    phase: descriptor.availability === "unsupported" ? "unsupported" : "unavailable",
    source: descriptor.source,
    reason: descriptor.reason ?? descriptor.availability,
  };
}

function sourceRevision(source: DiffSourceRef): string {
  if (source.source === "baseline" && source.contentRevision === "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855") {
    return "首次新增 · 旧文档不存在";
  }
  const version = source.version ? `版本 ${source.version}` : "未绑定平台版本";
  const modified = source.modifiedAt === null ? "时间未知" : auditMoment(source.modifiedAt);
  return `${SOURCE_LABELS[source.source]} · ${version} · ${modified} · ${shortRevision(source.contentRevision)}`;
}

function shortRevision(revision: string): string {
  return `SHA-256 ${revision.slice(0, 12)}`;
}

function extractDiffReason(error: unknown): string {
  const value = String(error);
  return Object.keys(DIFF_ERROR_LABELS).find((reason) => value.includes(reason)) ?? "text_diff_read_failed";
}
