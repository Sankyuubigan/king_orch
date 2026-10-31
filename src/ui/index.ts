// 🚪 ПУБЛИЧНЫЙ КОНТРАКТ UI-модуля
// Контроллеры могут импортировать ТОЛЬКО отсюда

export { createMessageElement, createThoughtElement, createToolThoughtElement, createSubcallElement, createToolCallElement } from './render'
export type { Role } from './render'

export { createThoughtsBlock, addToThoughtsBlock } from './thoughts-block'

export { createProcessingIndicator } from './processing-indicator'
export type { ProcessingIndicator } from './processing-indicator'

export { showToast } from './toast'

export { confirmDialog, initConfirmDialog } from './confirm'

export { initPermissionDialog, showPermissionRequest } from './permission'

// VRAM-модалка убрана (tasks/31.10.26): уведомления плагина о памяти идут
// системным сообщением в чат (ChatController.onVramNotice ← chat-router.ts).

export { createMessageMenu } from './message-menu'
export type { MessageMenuCallbacks } from './message-menu'

export { initImageViewer, openImageViewer } from './image-viewer'

export { createImageAttachmentElement } from './image-attachment'
export type { ImageAttachmentCallbacks } from './image-attachment'
export { createWorkspaceMenu } from './workspace-menu'
export type { WorkspaceMenuAction, WorkspaceMenuController } from './workspace-menu'