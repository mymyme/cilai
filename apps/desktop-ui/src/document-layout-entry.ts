const layout = (window as unknown as { cilaiDocumentLayout: {
  take(): Promise<{ format: "docx"; input: ArrayBuffer } | { format: "html"; input: string }>;
  complete(html: string): Promise<unknown>;
  fail(): Promise<unknown>;
} }).cilaiDocumentLayout;

const documentPolicy = "default-src 'none'; img-src data: blob:; font-src data: blob:; style-src 'unsafe-inline'; script-src 'none'; connect-src 'none'; object-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'";
const metaPolicy = `<meta http-equiv="Content-Security-Policy" content="${documentPolicy}">`;
const staticStyle = `<style data-contexthub-static-preview>
.reveal,[data-aos],[data-scroll-reveal]{opacity:1!important;visibility:visible!important;transform:none!important;transition:none!important}
</style>`;

function htmlDocument(input: string) {
  const parsed = new DOMParser().parseFromString(input, "text/html");
  parsed.head.insertAdjacentHTML("afterbegin", metaPolicy);
  parsed.head.insertAdjacentHTML("beforeend", staticStyle);
  for (const item of parsed.querySelectorAll("meta[http-equiv=refresh i],base")) item.remove();
  for (const anchor of parsed.querySelectorAll("a[href],area[href]")) {
    const target = anchor.getAttribute("href")?.trim();
    if (target?.startsWith("#")) {
      anchor.setAttribute("href", `about:srcdoc${target}`);
      anchor.setAttribute("target", "_self");
    } else anchor.removeAttribute("href");
  }
  return `<!doctype html>${parsed.documentElement.outerHTML}`;
}

async function wordDocument(bytes: ArrayBuffer) {
  const body = document.createElement("div");
  const styles = document.createElement("div");
  const { renderAsync } = await import("docx-preview");
  await renderAsync(bytes, body, styles, {
    useBase64URL: true, renderAltChunks: false, renderComments: false,
    renderChanges: false, ignoreLastRenderedPageBreak: false, experimental: false,
  });
  for (const element of body.querySelectorAll("a,form,script,iframe,object,embed,input,button")) {
    if (element.tagName === "A") element.replaceWith(...element.childNodes);
    else element.remove();
  }
  return `<!doctype html><html><head><meta charset="utf-8">${metaPolicy}<style>body{margin:0;background:#f4f4f3}.docx-wrapper{padding:20px!important}section.docx{max-width:100%;box-sizing:border-box}</style>${styles.innerHTML}</head><body>${body.innerHTML}</body></html>`;
}

void layout.take().then(async job => layout.complete(job.format === "html" ? htmlDocument(job.input) : await wordDocument(job.input))).catch(() => layout.fail());
