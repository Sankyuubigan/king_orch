export function renderMarkdown(text: string): string {
  if (!text) return "";
  let html = text.replace(/<think>[\s\S]*?<\/think>/gi, '');
  
  // Вырезаем блоки mermaid до экранирования HTML, чтобы сохранить их синтаксис
  const mermaidBlocks: string[] = [];
  html = html.replace(/```mermaid\s*([\s\S]*?)```/gim, (_, code) => {
    mermaidBlocks.push(code.trim());
    return `__MERMAID_BLOCK_${mermaidBlocks.length - 1}__`;
  });

  html = html.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

  html = html.replace(/^### (.*$)/gim, '<h3>$1</h3>');
  html = html.replace(/^## (.*$)/gim, '<h2>$1</h2>');
  html = html.replace(/^# (.*$)/gim, '<h1>$1</h1>');
  html = html.replace(/\*\*(.*?)\*\*/gim, '<strong>$1</strong>');
  html = html.replace(/\*(.*?)\*/gim, '<em>$1</em>');
  html = html.replace(/^\s*[-*]\s+(.*$)/gim, '<li>$1</li>');
  html = html.replace(/(<li>.*?<\/li>(\s*<li>.*?<\/li>)*)/gims, '<ul>$1</ul>');

  html = html.split('\n').map(line => {
    if (line.trim() === '' || line.startsWith('<h') || line.startsWith('<ul') || line.startsWith('<li') || line.startsWith('</ul') || line.startsWith('__MERMAID_BLOCK_')) {
      return line;
    }
    return `${line}<br>`;
  }).join('\n');

  // Возвращаем чистые mermaid блоки обратно в HTML
  mermaidBlocks.forEach((code, index) => {
    html = html.replace(`__MERMAID_BLOCK_${index}__`, `<pre class="mermaid">${code}</pre>`);
  });

  return `<div class="markdown-body">${html}</div>`;
}