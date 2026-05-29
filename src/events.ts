/**
 * Шина событий для loose coupling между модулями.
 * Контроллеры общаются через события, а не прямыми вызовами.
 * Это позволяет менять/удалять модули независимо.
 */
type Handler = (...args: any[]) => void;

class EventBus {
  private handlers = new Map<string, Set<Handler>>();

  on(event: string, fn: Handler): () => void {
    if (!this.handlers.has(event)) this.handlers.set(event, new Set());
    this.handlers.get(event)!.add(fn);
    return () => this.handlers.get(event)?.delete(fn);
  }

  emit(event: string, ...args: any[]) {
    this.handlers.get(event)?.forEach(fn => fn(...args));
  }
}

export const bus = new EventBus();