function collectDocument(selector, linkSelector, attachmentCapture = false) {
  const selected = [...document.querySelectorAll(selector)];
  const nodes = selected.filter(node => !selected.some(parent => parent !== node && parent.contains(node)));
  if (!nodes.length) throw new Error("未找到正文元素");
  const text = value => String(value ?? "").replace(/[\\`*_{}[\]()#+!|>]/g, "\\$&");
  const address = value => {
    try {
      const url = new URL(value, location.href);
      return ["https:", "http:"].includes(url.protocol) && !url.username && !url.password ? url.href : null;
    } catch { return null; }
  };
  const children = node => [...node.childNodes].map(convert).join("");
  function convert(node) {
    if (node.nodeType === 3) return text(node.textContent).replace(/\s+/g, " ");
    if (node.nodeType !== 1) return "";
    const tag = node.tagName.toLowerCase();
    if (["script", "style", "noscript", "template", "nav", "button", "input"].includes(tag) || node.hidden || node.getAttribute("aria-hidden") === "true") return "";
    if (tag === "table") {
      const rows = [...node.querySelectorAll("tr")].map(row => [...row.cells].map(cell => children(cell).replace(/\n+/g, " ").trim()));
      if (!rows.length) return "";
      const width = Math.max(...rows.map(row => row.length));
      const line = row => `| ${Array.from({length:width}, (_, i) => row[i] ?? "").join(" | ")} |`;
      return `\n\n${line(rows[0])}\n${line(Array(width).fill("---"))}\n${rows.slice(1).map(line).join("\n")}\n\n`;
    }
    if (tag === "br") return "\n";
    if (tag === "a") {
      const href = address(node.getAttribute("href"));
      return href ? `[${children(node).trim()}](<${href.replace(/>/g, "%3E")}>)` : children(node);
    }
    if (tag === "img") {
      const src = address(node.getAttribute("src"));
      return src ? `![${text(node.getAttribute("alt"))}](<${src.replace(/>/g, "%3E")}>)` : "";
    }
    if (/^h[1-6]$/.test(tag)) return `\n\n${"#".repeat(Number(tag[1]))} ${children(node).trim()}\n\n`;
    if (tag === "li") return `\n- ${children(node).trim()}`;
    if (tag === "strong" || tag === "b") return `**${children(node).trim()}**`;
    const content = children(node);
    return ["p", "div", "section", "article", "ul", "ol", "blockquote"].includes(tag) ? `\n\n${content.trim()}\n\n` : content;
  }
  const markdown = nodes.map(convert).join("\n\n").replace(/\n{3,}/g, "\n\n").trim();
  if (!markdown) throw new Error("页面正文为空");
  if (markdown.length > 200000) throw new Error("页面正文超过采集限额");
  const metadata = name => document.querySelector(`meta[name="${name}"],meta[property="${name}"]`)?.content?.trim() ?? "";
  const title = metadata("ArticleTitle") || document.title;
  const dateValue = metadata("PubDate") || metadata("article:published_time");
  const published = /^\d{4}-\d{2}-\d{2}/.test(dateValue) ? dateValue.slice(0,10) : "";
  const attachments = [], attachmentUrls = new Set();
  for (const node of nodes.flatMap(root => [...root.querySelectorAll("a[href],iframe[src],embed[src],object[data]")])) {
    if (node.closest('[hidden],[aria-hidden="true"]')) continue;
    const url = address(node.getAttribute("href") || node.getAttribute("src") || node.getAttribute("data"));
    if (!url) continue;
    const filePath = new URL(url).pathname;
    let fileName;
    try { fileName = decodeURIComponent(filePath.slice(filePath.lastIndexOf("/") + 1)); }
    catch { throw new Error("附件文件名编码无效"); }
    const downloaded = node.getAttribute("download"), type = node.getAttribute("type") || "", label = node.textContent.trim();
    const fileExtension = /\.(pdf|docx?|txt|md|png|jpe?g|xlsx?|pptx?|zip|rar|7z|odt|ods|rtf)$/i;
    const declaredAttachment = /^(?:附件|下载附件|附表)(?:下载|[:：\s（(一二三四五六七八九十\d]|$)/.test(label);
    if (!fileExtension.test(fileName) && !fileExtension.test(label) && !declaredAttachment && downloaded === null && !/^(application\/(pdf|msword|vnd\.|zip)|image\/)/i.test(type)) continue;
    if (attachmentUrls.has(url)) continue;
    attachmentUrls.add(url);
    attachments.push({url, name: downloaded || (fileExtension.test(label) ? label : fileName) || label});
  }
  if (attachments.length && attachmentCapture !== true) throw new Error("当前采集执行端不支持附件原件，请升级工作台");
  if (attachments.length > 8) throw new Error("页面附件超过八项，请缩小采集范围");
  const links = linkSelector ? [...new Set([...document.querySelectorAll(linkSelector)].map(n => address(n.href)).filter(url => url && !attachmentUrls.has(url)))] : [];
  if (links.length > 30) throw new Error("页面链接超过三十项，请缩小采集范围");
  return {url:location.href,title,text:markdown,published,links,attachments};
}
module.exports = { collectDocument };
