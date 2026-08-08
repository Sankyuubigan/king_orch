/**
 * 🛰️ Слой телеметрии фронтенда — единственное место, откуда уходят
 * события в сервис сбора ошибок (сейчас Aptabase через Tauri-команду
 * `plugin:aptabase|track_event`).
 *
 * 🔄 Смена сервиса = переписать только этот файл.
 *
 * Уважает настройку «Отправлять анонимные отчёты об ошибках»:
 * если юзер выключил — слушатели не вешаются, события не отправляются,
 * Tauri-команда не вызывается вовсе.
 */
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";

let enabled = false;
let listenersAttached = false;
let appVersion = "";

// Версия приложения для контекста событий (некритично, если недоступна).
getVersion()
  .then((v) => { appVersion = v; })
  .catch(() => { /* версия некритична — события уйдут без неё */ });

/** Включена ли отправка телеметрии (по настройке пользователя). */
export function isTelemetryEnabled(): boolean {
  return enabled;
}

/** Принудительно включить/выключить отправку (при переключении галочки). */
export function setTelemetryEnabled(v: boolean): void {
  enabled = v;
}

/**
 * Инициализация при старте: читает настройку приложения и, если юзер не
 * против, вешает глобальные ловушки ошибок UI (window error +
 * unhandledrejection). Вызывается в main.ts самой первой.
 */
export async function initTelemetry(): Promise<void> {
  try {
    const config: any = await invoke("get_config");
    enabled = config.allow_error_reports !== false;
  } catch {
    // Не смогли прочитать настройку — консервативно OFF, ничего не шлём.
    enabled = false;
  }
  if (!enabled || listenersAttached) return;
  listenersAttached = true;

  // Необработанная ошибка / краш UI → "Frontend Error"
  window.addEventListener("error", (e) => {
    const message = e.message || (e.error instanceof Error ? e.error.message : "unknown error");
    const stack = e.error instanceof Error ? e.error.stack || "" : "";
    void trackEvent("Frontend Error", { message, stack });
  });

  // Необработанное отклонение промиса → "Frontend Error"
  window.addEventListener("unhandledrejection", (e) => {
    const reason: unknown = e.reason;
    const message = reason instanceof Error ? reason.message : String(reason);
    const stack = reason instanceof Error ? reason.stack || "" : "";
    void trackEvent("Frontend Error", { message, stack });
  });
}

/**
 * Отправить событие. Никогда не бросает и не ломает UI: ошибки отправки
 * только логируются в консоль (дублировать их в лог-вкладку не нужно —
 * это же сами отчёты об ошибках).
 */
export async function trackEvent(
  name: string,
  props?: Record<string, string | number>,
): Promise<void> {
  if (!enabled) return;
  try {
    const fullProps: Record<string, string | number> = { ...(props ?? {}) };
    if (appVersion) fullProps.app_version = appVersion;
    await invoke("plugin:aptabase|track_event", { name, props: fullProps });
  } catch (err) {
    console.warn(`[telemetry] trackEvent("${name}") не удалось:`, err);
  }
}

/**
 * Отследить ОБРАБОТАННУЮ ошибку UI (catch в контроллерах).
 * Никогда не бросает и не ломает интерфейс.
 */
export async function trackError(whence: string, err: unknown): Promise<void> {
  const message = err instanceof Error ? err.message : String(err ?? "unknown error");
  const stack = err instanceof Error ? (err.stack || "") : "";
  await trackEvent("UI Error", {
    when: whence,
    message,
    stack: stack.slice(0, 2000),
  });
}