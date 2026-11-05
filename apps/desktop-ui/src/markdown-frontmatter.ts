type FrontmatterEntry = readonly [key: string, value: string];

const FRONTMATTER_BLOCK = /^---[ \t]*\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)/;
const FRONTMATTER_KEY = /^([A-Za-z0-9_][\w.-]*):[ \t]*(.*)$/;

/**
 * 技能说明等文件以 YAML 属性块开头。不拆出来的话，结尾那行 --- 会让整段被当成二级标题，
 * 渲染成一大段粗体。这里只做展示用的浅解析：顶层"键: 值"各占一行，缩进行并入上一个键。
 * 解析不出任何键时原样交给 Markdown，不猜测格式。
 */
export function splitFrontmatter(source: string): { frontmatter: FrontmatterEntry[] | null; body: string } {
  const match = FRONTMATTER_BLOCK.exec(source);
  if (!match) return { frontmatter: null, body: source };
  const entries: Array<[string, string]> = [];
  for (const line of match[1].split(/\r?\n/)) {
    const keyed = /^\s/.test(line) ? null : FRONTMATTER_KEY.exec(line);
    if (keyed) {
      entries.push([keyed[1], unquote(keyed[2].trim())]);
    } else if (entries.length && line.trim()) {
      const [key, value] = entries[entries.length - 1];
      entries[entries.length - 1] = [key, value ? `${value}\n${line.trim()}` : line.trim()];
    }
  }
  if (!entries.length) return { frontmatter: null, body: source };
  return { frontmatter: entries, body: source.slice(match[0].length) };
}

function unquote(value: string): string {
  return /^(["']).*\1$/.test(value) ? value.slice(1, -1) : value;
}
