export function skillMessageText(content: string): string {
  const lines = content.match(/[^\n]*(?:\n|$)/g) ?? [];
  let opening: { marker: string; size: number; target: boolean; start: number; body: number } | null = null;
  let block: { start: number; end: number; body: string } | null = null;
  let offset = 0;
  for (const line of lines) {
    const fence = /^ {0,3}(`{3,}|~{3,})([^\r\n]*)\r?\n?$/.exec(line);
    if (fence) {
      const marker = fence[1][0];
      const size = fence[1].length;
      const info = fence[2].trim();
      if (opening) {
        if (opening.marker === marker && size >= opening.size && !info) {
          if (opening.target) {
            if (block) return content;
            block = { start: opening.start, end: offset + line.length, body: content.slice(opening.body, offset) };
          }
          opening = null;
        }
      } else opening = { marker, size, target: info.toLowerCase() === "cilai-skill", start: offset, body: offset + line.length };
    }
    offset += line.length;
  }
  if (!block || opening?.target) return content;
  try {
    const draft = JSON.parse(block.body) as Record<string, unknown>;
    if (typeof draft.name !== "string" || typeof draft.description !== "string" || typeof draft.instructions !== "string" || !Array.isArray(draft.files)) return content;
  } catch { return content; }
  return (content.slice(0, block.start) + content.slice(block.end)).trim();
}
