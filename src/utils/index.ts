// 🚪 ПУБЛИЧНЫЙ КОНТРАКТ утилит
export { renderMarkdown } from './markdown'
export { stripStreamArtifacts, extractChannelThought } from './stream-filter'
export { buildChatPage, buildWebviewPage } from './chat-dom'
export type { ChatPageElements, SharedChatControls } from './chat-dom'
export { buildSessionMarkdown, copyMarkdownToClipboard } from './session-clipboard'

// Константа и заполнители селектов моделей/агентов перенесены в
// ./model-options (чтобы не тянуть store в leaf-утилиту), ре-экспортируются
// отсюда для обратной совместимости.
export {
  NINE_ROUTER_MODEL_PREFIX,
  fillModelSelect,
  fillAgentSelect,
  renderNineRouterOptions,
  getAgentDisplayName,
} from './model-options'

export function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${Math.round(bytes)} B`;
}

export function formatSpeed(bps?: number): string {
  if (bps == null || !isFinite(bps) || bps < 0) return "0 B/s";
  if (bps === 0) return "0 B/s";
  return `${formatBytes(bps)}/s`;
}