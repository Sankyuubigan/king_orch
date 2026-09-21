// 🚪 ПУБЛИЧНЫЙ КОНТРАКТ утилит
export { renderMarkdown } from './markdown'
export { stripStreamArtifacts, extractChannelThought } from './stream-filter'

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

/// Префикс значения комбо 9Router в списке моделей (см. SettingsController).
/// Выбор с этим префиксом идёт в облачный шлюз 9Router (плагин
/// tauri-plugin-9router), а НЕ в локальный llama-server.
export const NINE_ROUTER_MODEL_PREFIX = "9router:";