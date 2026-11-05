import { Children, Fragment, isValidElement, memo, useEffect, useId, useMemo, useRef, useState, type MouseEvent, type ReactNode } from "react";
import { defaultUrlTransform, type Components } from "react-markdown";
import { ConversationMarkdown } from "@cilai/app-ui/conversation";
import { splitFrontmatter } from "./markdown-frontmatter";
export { splitFrontmatter } from "./markdown-frontmatter";

export const MarkdownContent = memo(function MarkdownContent({ children, diagramIdPrefix = "markdown", canOpenLink, onContextLink, onOpenLink }: {
  children: string;
  diagramIdPrefix?: string;
  canOpenLink?: (href: string) => boolean;
  onContextLink?: (event: MouseEvent<HTMLAnchorElement>, href: string) => void;
  onOpenLink?: (href: string) => void;
}) {
  const linkHandlers = useRef({ canOpenLink, onContextLink, onOpenLink });
  linkHandlers.current = { canOpenLink, onContextLink, onOpenLink };
  const components = useMemo<Components>(() => ({
      a: ({ children: linkChildren, href }) => href && linkHandlers.current.canOpenLink?.(href)
        ? <a className="markdown-link" href={href} onClick={(event) => { event.preventDefault(); linkHandlers.current.onOpenLink?.(href); }} onContextMenu={(event) => linkHandlers.current.onContextLink?.(event, href)} title={href}>{linkChildren}</a>
        : <span className="markdown-link">{linkChildren}</span>,
      img: ({ alt }) => <span className="markdown-image-placeholder">[图片：{alt ?? "无说明"}]</span>,
      pre: ({ children: preChildren }) => {
        const child = Children.toArray(preChildren)[0];
        if (isValidElement<{ className?: string; children?: ReactNode }>(child)
          && child.props.className?.split(" ").includes("language-mermaid")) {
          return <MermaidDiagram idPrefix={diagramIdPrefix} source={Children.toArray(child.props.children).join("").replace(/\n$/, "")}/>;
        }
        return <pre>{preChildren}</pre>;
      },
      table: ({ children: tableChildren }) => <div className="markdown-table-scroll"><table>{tableChildren}</table></div>,
    }), [diagramIdPrefix]);
  const { frontmatter, body } = useMemo(() => splitFrontmatter(children), [children]);
  const markdown = <ConversationMarkdown
    components={components}
    urlTransform={(url, key) => key === "href" && /^(?:file:\/\/|[a-z]:(?:[\\/]|%5c|%2f))/i.test(url) && linkHandlers.current.canOpenLink?.(url) ? url : defaultUrlTransform(url)}
    skipHtml
  >
    {body}
  </ConversationMarkdown>;
  if (!frontmatter) return markdown;
  return <>
    <dl aria-label="文档属性" className="markdown-frontmatter">
      {frontmatter.map(([key, value], index) => <Fragment key={`${key}-${index}`}><dt>{key}</dt><dd>{value}</dd></Fragment>)}
    </dl>
    {markdown}
  </>;
});

let mermaidPromise: Promise<(typeof import("mermaid"))["default"]> | null = null;

function loadMermaid() {
  mermaidPromise ??= import("mermaid").then(({ default: mermaid }) => {
    mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: "neutral", layout: "dagre", look: "classic" });
    return mermaid;
  });
  return mermaidPromise;
}

export function MermaidDiagram({ idPrefix, source }: { idPrefix: string; source: string }) {
  const container = useRef<HTMLDivElement>(null);
  const diagramId = `${idPrefix}-mermaid-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
  const [error, setError] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setError(false);
    container.current?.replaceChildren();
    void loadMermaid()
      .then((mermaid) => mermaid.render(diagramId, source))
      .then(({ svg }) => {
        if (!cancelled && container.current) container.current.innerHTML = svg;
      })
      .catch(() => {
        if (!cancelled) setError(true);
      });
    return () => {
      cancelled = true;
      container.current?.replaceChildren();
    };
  }, [diagramId, source]);
  return <figure className="mermaid-diagram">
    {error ? <figcaption role="alert">Mermaid 图表语法有误，无法渲染。</figcaption> : <div aria-label="Mermaid 图表" ref={container}/>} 
  </figure>;
}
