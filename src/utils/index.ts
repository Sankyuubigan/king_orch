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
  if (!bps || !isFinite(bps) || bps <= 0) return "";
  return `${formatBytes(bps)}/s`;
}