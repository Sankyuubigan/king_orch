// 🚪 ПУБЛИЧНЫЙ КОНТРАКТ UI-модуля
// Контроллеры могут импортировать ТОЛЬКО отсюда

export { createMessageElement, createThoughtElement, createToolThoughtElement, createSubcallElement, createToolCallElement } from './render'
export type { Role } from './render'

export { createThoughtRow, updateThoughtRowSummary, completeToolRow, TOOL_ICONS } from './thought-row'
export type { ThoughtRowKind, ThoughtRowOptions } from './thought-row'

export { createProcessingIndicator } from './processing-indicator'
export type { ProcessingIndicator } from './processing-indicator'

export { showToast } from './toast'

export { confirmDialog, initConfirmDialog } from './confirm'

export { initPermissionDialog, showPermissionRequest } from './permission'

// VRAM-модалка убрана (tasks/31.10.26): уведомления плагина о памяти идут
// системным сообщением в чат (ChatController.onVramNotice ← chat-router.ts).

export { createMessageMenu, setActiveDropdown } from './message-menu'
export type { MessageMenuCallbacks } from './message-menu'

export { materializeSchemeCards } from './scheme-card'
export type { MaterializeSchemeCardsOptions } from './scheme-card'
export { ensureSchemesRegistered, initSchemeRegistry } from './scheme-registry'
export { attachMermaidSchemeOpen, openSchemeViewer } from './scheme-viewer'
export type { SchemeViewerRequest } from './scheme-viewer'
export { initImageViewer, openImageViewer } from './image-viewer'

export { createImageAttachmentElement } from './image-attachment'
export type { ImageAttachmentCallbacks } from './image-attachment'

export { createAttachmentCallbacks } from './attachment-actions'

export { createPreviewAttachmentRow } from './preview-attachments'
export type { VideoFrameCache } from './preview-attachments'

export { extractVideoFirstFrame, extractVideoFirstFrameFromDataUrl, extractVideoFirstFrameFromPath } from './video-frame'

export { createWorkspaceMenu } from './workspace-menu'
export type { WorkspaceMenuAction, WorkspaceMenuController } from './workspace-menu'