import { showToast } from "../ui";
import { trackError } from "../telemetry";

/// Собирает Markdown-представление переписки (общий формат «Копировать в буфер»).
export function buildSessionMarkdown(title: string, messages: any[]): string {
  let out = `# ${title}\n\n`;
  for (const msg of messages) {
    if (!msg || msg.type !== "message") continue;
    const content = (msg.content ?? "").trim();
    if (!content) continue;
    let role = "Ассистент";
    if (msg.author === "user") role = "Пользователь";
    else if (msg.author === "system") role = "Система";
    else if (msg.author) role = String(msg.author);
    out += `### ${role}\n\n${content}\n\n`;
  }
  return out.trim() + "\n";
}

/// Копирует переписку в буфер обмена (Markdown). `messages` на месте вызова
/// уже готовы (загрузка с диска или живой стейт открытой вкладки).
export async function copyMarkdownToClipboard(title: string, messages: any[]): Promise<void> {
  try {
    const md = buildSessionMarkdown(title, messages);
    await navigator.clipboard.writeText(md);
    showToast("Переписка скопирована в буфер", "success");
  } catch (err) {
    showToast(`Ошибка: ${err}`, "error");
    void trackError("sessionClipboard.copy", err);
    throw err;
  }
}