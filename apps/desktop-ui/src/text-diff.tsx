import { invoke } from "./host";
import { useCallback, useEffect, useState } from "react";
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
import { ERROR_LABELS, SOURCE_LABELS } from "./preview";

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
}

export function FileDiffPane({
  workspaceId,
  localFileId,
  leftSource,
  rightSource,
  onReadyChange,
}: FileDiffPaneProps) {
  const { state, loadMore } = useTextDiff({ workspaceId, localFileId, leftSource, rightSource });
  useEffect(() => {
    onReadyChange?.(state.phase === "ready");
  }, [onReadyChange, state.phase]);
  return (
    <section className="diff-pane" aria-label="文本差异">
      <header className="diff-source-header">
        <SourceHeader side="旧文档" source={leftSource} />
        <SourceHeader side="新文档" source={rightSource} />
      </header>
      <DiffBody loadMore={loadMore} state={state} />
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

function DiffBody({ state, loadMore }: { state: DiffState; loadMore: () => void }) {
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
}: {
  title: string;
  detail?: string;
  source?: PreviewSource;
}) {
  return (
    <div className="preview-message" role="status">
      {source ? <span className="preview-source-label">{SOURCE_LABELS[source]}</span> : null}
      <h3>{title}</h3>
      {detail ? <p>{detail}</p> : null}
    </div>
  );
}

function useTextDiff(params: {
  workspaceId: string;
  localFileId: string;
  leftSource: PreviewSource;
  rightSource: PreviewSource;
}): { state: DiffState; loadMore: () => void } {
  const [state, setState] = useState<DiffState>({ phase: "loading" });
  useEffect(() => {
    let cancelled = false;
    setState({ phase: "loading" });
    void loadTextDiff(params)
      .then((next) => {
        if (!cancelled) setState(next);
      })
      .catch((error: unknown) => {
        if (!cancelled) setState({ phase: "error", reason: extractDiffReason(error) });
      });
    return () => {
      cancelled = true;
    };
  }, [params.leftSource, params.localFileId, params.rightSource, params.workspaceId]);
  const loadMore = useCallback(() => {
    if (state.phase !== "ready" || state.nextCursor === null || state.loadingMore) return;
    const { result, nextCursor } = state;
    setState({ ...state, loadingMore: true });
    void readDiffPage(result.diffTicketId, nextCursor)
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
        setState((current) =>
          current.phase === "ready" && current.result.diffTicketId === result.diffTicketId
            ? { phase: "error", reason: extractDiffReason(error) }
            : current,
        );
      });
  }, [state]);
  return { state, loadMore };
}

async function loadTextDiff(params: {
  workspaceId: string;
  localFileId: string;
  leftSource: PreviewSource;
  rightSource: PreviewSource;
}): Promise<DiffState> {
  const [left, right] = await Promise.all([
    openSource(params.workspaceId, params.localFileId, params.leftSource),
    openSource(params.workspaceId, params.localFileId, params.rightSource),
  ]);
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
  const page = await readDiffPage(result.diffTicketId, 0);
  return {
    phase: "ready",
    result,
    rows: page.rows,
    nextCursor: page.nextCursor,
    loadingMore: false,
  };
}

async function readDiffPage(diffTicketId: string, cursor: number): Promise<ReadTextDiffResult> {
  const params: ReadTextDiffParams = {
    diffTicketId,
    cursor,
    maxRows: 200,
    maxBytes: 256 * 1024,
  };
  const page = await invoke<ReadTextDiffResult>("text_diff_read", { params });
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
