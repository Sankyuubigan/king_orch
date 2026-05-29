// 🚪 ПУБЛИЧНЫЙ КОНТРАКТ UI-модуля
// Контроллеры могут импортировать ТОЛЬКО отсюда

export { createMessageElement, createThoughtElement, createSubcallElement, createToolCallElement } from './render'
export type { Role } from './render'

export { createThoughtsBlock, addToThoughtsBlock } from './thoughts-block'

export { showToast } from './toast'

export { confirmDialog, initConfirmDialog } from './confirm'

export { createMessageMenu } from './message-menu'
export type { MessageMenuCallbacks } from './message-menu'