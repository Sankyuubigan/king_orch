// 🚪 ПУБЛИЧНЫЙ КОНТРАКТ контроллеров
export {
  ChatController,
  ChatTabState,
  getActiveProcessingChat,
  releaseGlobalProcessing,
} from './chat'
export type { ChatElements, ChatHooks } from './chat'
export { initChatEventRouter } from './chat-router'
export { TabController } from './tabs'

export { SessionController } from './sessions'
export type { SessionElements } from './sessions'

export { SettingsController } from './settings'
export type { SettingsElements } from './settings'

export type { GraphController, GraphElements } from './graph'

export async function loadGraphController(): Promise<typeof import('./graph').GraphController> {
  return (await import('./graph')).GraphController;
}

export { AgentTestController } from './agent-test'
export type { AgentTestElements } from './agent-test'

export { CodingTestController } from './coding-test'
export type { CodingTestElements } from './coding-test'

export { UpdatePopupController } from './update_popup'
export type { UpdatePopupElements } from './update_popup'