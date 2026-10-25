import { useEffect, useMemo, useRef, useState } from "react";
import type { UnifiedSearchGroup, UnifiedSearchItem, UnifiedSearchResult } from "./generated/protocol";
import { searchSources } from "./search-sources";
import type { MessageSink } from "./app-message";
import { hostPlatformUi } from "./host-platform-ui";
import { LoadFailure } from "./load-failure";
import { SearchField } from "./search-field";
import "./global-search.css";

const sourceOptions = [
  { id: "all", label: "全部" },
  { id: "conversations", label: "对话" },
  { id: "local", label: "本地工作" },
] as const;
type SearchSource = typeof sourceOptions[number]["id"];

const typeLabels: Record<string, string> = {
  conversation: "对话",
  project: "项目",
  phase: "阶段",
  task: "任务",
  deliverable: "交付物",
  decision: "决策",
  file: "文件",
  document: "文档",
  report: "报告",
  analysis: "分析",
};

const failureLabels: Record<string, string> = {
  "native-conversations": "对话",
  "native-archived-conversations": "已归档对话",
  local: "本地工作",
};

interface GlobalSearchProps {
  profileId: string;
  hostPlatform: string | null;
  onMessage: MessageSink;
  onOpen: (item: UnifiedSearchItem) => void;
}

export function GlobalSearchPage(props: GlobalSearchProps) {
  return <SearchPage key={props.profileId} {...props}/>;
}

function SearchPage(props: GlobalSearchProps) {
  const [query, setQuery] = useState("");
  const [source, setSource] = useState<SearchSource>("all");
  const [limit, setLimit] = useState(8);
  const [result, setResult] = useState<UnifiedSearchResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<unknown>(null);
  const [retryNonce, setRetryNonce] = useState(0);
  const [paging, setPaging] = useState<string | null>(null);
  const [pageErrors, setPageErrors] = useState<Record<string, unknown>>({});
  const pagingRef = useRef(false);
  const generation = useRef(0);
  const input = useRef<HTMLInputElement>(null);
  const platformUi = hostPlatformUi(props.hostPlatform);

  useEffect(() => { input.current?.focus(); }, []);
  useEffect(() => {
    const normalized = query.trim();
    const current = ++generation.current;
    setResult(null);
    setLoadError(null);
    setPageErrors({});
    setPaging(null);
    pagingRef.current = false;
    if (normalized.length < 2) {
      setLoading(false);
      return;
    }
    setLoading(true);
    const timer = window.setTimeout(() => {
      void searchSources({
        query: normalized,
        scope: source === "all" ? ["conversations", "local"] : [source],
        limit,
        cursors: [],
      })
        .then((value) => {
          if (generation.current === current) setResult(value);
        })
        .catch((error) => {
          if (generation.current !== current) return;
          if (String(error).includes("unified_search_superseded")) return;
          setLoadError(error);
        })
        .finally(() => {
          if (generation.current === current) setLoading(false);
        });
    }, 320);
    return () => { window.clearTimeout(timer); generation.current += 1; };
  }, [query, source, limit, retryNonce]);

  const groups = useMemo(
    () => (result?.groups ?? []).filter((group) => group.items.length > 0 || group.nextCursor),
    [result],
  );
  const total = useMemo(
    () => groups.reduce((count, group) => count + group.items.length, 0),
    [groups],
  );
  const failures = result?.partialFailures ?? [];
  const normalized = query.trim();

  const loadMore = async (group: UnifiedSearchGroup) => {
    if (!group.nextCursor || pagingRef.current) return;
    const current = generation.current;
    pagingRef.current = true;
    setPaging(group.groupId);
    setPageErrors((errors) => { const next = { ...errors }; delete next[group.groupId]; return next; });
    try {
      if (group.kind !== "local" && group.kind !== "conversations") throw new Error("此搜索来源已停用，请重新搜索。");
      const page = await searchSources({
        query: normalized, scope: [group.kind], limit,
        cursors: [{ providerId: group.providerId, value: group.nextCursor }],
      });
      if (generation.current !== current) return;
      const failure = page.partialFailures.find((item) => item.providerId === group.providerId);
      if (failure) throw new Error(`${failure.message} (${failure.code})`);
      const nextGroup = page.groups.find((item) => item.groupId === group.groupId && item.providerId === group.providerId);
      if (!nextGroup || nextGroup.nextCursor === group.nextCursor) throw new Error("分页返回无效，请重新搜索。");
      setResult((previous) => previous ? { ...previous, requestId: page.requestId, groups: previous.groups.map((item) => {
        if (item.groupId !== group.groupId || item.providerId !== group.providerId) return item;
        const seen = new Set(item.items.map((hit) => `${hit.providerId}:${hit.objectType}:${hit.objectId}`));
        return { ...nextGroup, items: [...item.items, ...nextGroup.items.filter((hit) => {
          const key = `${hit.providerId}:${hit.objectType}:${hit.objectId}`;
          if (seen.has(key)) return false;
          seen.add(key); return true;
        })] };
      }) } : previous);
    } catch (error) {
      if (generation.current === current) {
        if (String(error).includes("cursor_expired") || String(error).includes("授权已变化")) { setResult(null); setLoadError(error); }
        else setPageErrors((errors) => ({ ...errors, [group.groupId]: error }));
      }
    } finally {
      if (generation.current === current) { pagingRef.current = false; setPaging(null); }
    }
  };

  return <section className="global-search-page">
    <header className="global-search-header"><h2>全局搜索</h2></header>
    <div className="global-search-body">
      <SearchField
        ariaLabel="全局搜索"
        className="global-search-input"
        inputRef={input}
        onChange={(value) => { setQuery(value); setLimit(8); }}
        placeholder="搜索对话、项目、任务或文件正文"
        trailing={loading ? <span>检索中…</span> : platformUi ? <kbd>{platformUi.globalSearchShortcut}</kbd> : null}
        value={query}
      />
      <p className="global-search-text-scope">正文：UTF-8 文本（含 BOM），单文件 ≤ 2 MiB</p>
      <div aria-label="搜索来源" className="global-search-sources" role="group">
        {sourceOptions.map((option) => <button aria-pressed={source === option.id} key={option.id} onClick={() => { setSource(option.id); setLimit(8); }} type="button">{option.label}</button>)}
      </div>
      {normalized.length < 2 ? (
        <div className="global-search-empty"><SearchDocumentIcon/><strong>输入至少两个字符</strong></div>
      ) : loadError ? (
        <LoadFailure error={loadError} what="搜索结果" onRetry={() => setRetryNonce((value) => value + 1)}/>
      ) : (
        <>
          {failures.length ? <div aria-label="部分来源未完成" className="global-search-failures" role="status">
            {failures.map((failure) => <div key={`${failure.providerId}:${failure.groupId}`}>
              <strong>{failureLabels[failure.groupId] ?? failure.groupId}</strong>
              <span>{failure.message}</span>
            </div>)}
            <button onClick={() => setRetryNonce((value) => value + 1)} type="button">重试</button>
            {result?.requestId ? <small>请求 {result.requestId}</small> : null}
          </div> : null}
          {!loading && result && !total && !groups.some((group) => group.nextCursor) && !failures.length ? (
            <div className="global-search-empty"><SearchDocumentIcon/><strong>没有匹配结果</strong><span>尝试名称、文件路径或文本正文。</span></div>
          ) : groups.length ? (
            <div className="global-search-results">
              {groups.map((group) => <section key={`${group.providerId}:${group.groupId}`}>
                <header><strong>{group.label}</strong><span>{group.items.length}</span></header>
                {group.items.map((item) => <button aria-label={item.title} key={`${item.providerId}:${item.objectType}:${item.objectId}`} onClick={() => props.onOpen(item)} type="button">
                  <TypeIcon type={item.objectType}/>
                  <div>
                    <strong><SearchHighlight text={item.title} query={normalized}/></strong>
                    {item.summary ? <span><SearchHighlight text={item.summary} query={normalized}/></span> : null}
                    <small>{[typeLabels[item.objectType] ?? item.objectType, item.matchReason === "事实匹配" ? null : item.matchReason, item.localTarget?.relativePath].filter(Boolean).join(" · ")}</small>
                  </div>
                  <ArrowIcon/>
                </button>)}
                {pageErrors[group.groupId] ? <LoadFailure error={pageErrors[group.groupId]} what="后续结果" onRetry={() => void loadMore(group)}/> : null}
                {group.nextCursor && !pageErrors[group.groupId] ? <button className="global-search-more" disabled={paging !== null} onClick={() => void loadMore(group)} type="button">{paging === group.groupId ? "加载中…" : `加载更多${group.label}`}</button> : null}
              </section>)}
            </div>
          ) : null}
          {result && total > 0 ? <div className="global-search-range" role="status">
            <span>已显示 {total} 项</span>
            {groups.some((group) => group.items.length >= limit) ? limit < 20
              ? <button onClick={() => setLimit(20)} type="button">每组显示 20 项</button>
              : null : null}
          </div> : null}
        </>
      )}
    </div>
  </section>;
}

function SearchDocumentIcon() { return <svg aria-hidden="true" viewBox="0 0 48 48"><path d="M10 6h20l8 8v28H10zM30 6v9h8"/><circle cx="21" cy="27" r="6"/><path d="m25.5 31.5 5 5"/></svg>; }
function ArrowIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="m7 4 6 6-6 6"/></svg>; }
function TypeIcon(props: { type: string }) { return <svg aria-hidden="true" viewBox="0 0 24 24">{props.type === "file" || props.type.endsWith("_file") ? <><path d="M6 3h8l4 4v14H6zM14 3v5h5"/></> : props.type === "task" ? <><path d="M5 5h14v14H5zM8 9h8M8 13h5"/></> : props.type === "decision" ? <><path d="M5 3h14v18H5zM8 8h8M8 12h5"/><path d="m12 17 2 2 4-5"/></> : props.type === "deliverable" ? <><path d="M3 7h7l2 2h9v10H3z"/><path d="m8 14 2 2 5-5"/></> : <><circle cx="6" cy="6" r="2"/><circle cx="18" cy="18" r="2"/><path d="M8 6h5c3 0 5 2 5 5v5M6 8v10h10"/></>}</svg>; }

function SearchHighlight({ text, query }: { text: string; query: string }) {
  if (!query) return <>{text}</>;
  const normalized = text.toLocaleLowerCase();
  const needle = query.toLocaleLowerCase();
  const parts = [];
  let cursor = 0;
  let index = normalized.indexOf(needle);
  while (index !== -1) {
    parts.push(text.slice(cursor, index), <mark key={index}>{text.slice(index, index + query.length)}</mark>);
    cursor = index + query.length;
    index = normalized.indexOf(needle, cursor);
  }
  parts.push(text.slice(cursor));
  return <>{parts}</>;
}
