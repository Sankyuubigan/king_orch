export function renderMarkdown(text: string): string {
  if (!text) return "";
  let html = text.replace(/<think[\s\S]*?<\/think>/gi, '');
  
  // Удаление артефактов формата Gemma (</start_of_turn> и <start_of_turn>)
  html = html.replace(/<\/?start_of_turn>/gi, '');
  
  // Вырезаем блоки mermaid до экранирования HTML, чтобы сохранить их синтаксис
  const mermaidBlocks: string[] = [];
  html = html.replace(/```mermaid\s*([\s\S]*?)```/gim, (_, code) => {
    // Сразу очищаем LaTeX из mermaid-кода — Mermaid не понимает LaTeX
    let cleaned = code.trim();
    cleaned = cleaned.replace(/\$\\rightarrow\$/gi, '→');
    cleaned = cleaned.replace(/\$\\leftarrow\$/gi, '←');
    cleaned = cleaned.replace(/\$\\Rightarrow\$/gi, '⇒');
    cleaned = cleaned.replace(/\$\\Leftarrow\$/gi, '⇐');
    cleaned = cleaned.replace(/\$\\leftrightarrow\$/gi, '↔');
    cleaned = cleaned.replace(/\$\\Leftrightarrow\$/gi, '⇔');
    cleaned = cleaned.replace(/\$\\to\$/gi, '→');
    cleaned = cleaned.replace(/\$\\gets\$/gi, '←');
    cleaned = cleaned.replace(/\\rightarrow/gi, '→');
    cleaned = cleaned.replace(/\\leftarrow/gi, '←');
    cleaned = cleaned.replace(/\\Rightarrow/gi, '⇒');
    cleaned = cleaned.replace(/\\Leftarrow/gi, '⇐');
    cleaned = cleaned.replace(/\\to/gi, '→');
    cleaned = cleaned.replace(/\\gets/gi, '←');
    mermaidBlocks.push(cleaned);
    return `__MERMAID_BLOCK_${mermaidBlocks.length - 1}__`;
  });

  // Фикс для стрелок из синтаксиса LaTeX (в обычном тексте, не в mermaid)
  html = html.replace(/\$?\\rightarrow\$?/gi, '→');

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

  // Возвращаем mermaid блоки обратно в HTML (уже очищенные от LaTeX)
  mermaidBlocks.forEach((code, index) => {
    html = html.replace(`__MERMAID_BLOCK_${index}__`, `<pre class="mermaid">${code}</pre>`);
  });

  return `<div class="markdown-body">${html}</div>`;
}