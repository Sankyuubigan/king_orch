/**
 * 🛰️ Фронтенд-адаптер к единому runtime-плагину логов `tauri-plugin-logs`.
 *
 * Весь пайплайн живёт В ПЛАГИНЕ (core rules §2.5):
 * - локальный лог `king_orch.log` рядом с exe + dev-зеркало `test/last_logs.txt`;
 * - вкладка «Логи» как Web Component `<logs-panel>` (событие `logs:message`);
 * - опциональная облачная отправка (Aptabase, поле `project`),
 *   управляется флагом `set_reporting_enabled` (настройка allow_error_reports).
 *
 * Здесь — только стабильная обёртка для контроллеров, чтобы не менять
 * сигнатуры вызовов в 8 файлах. Ошибки UI ВСЕГДА пишутся в локальный лог,
 * в облако идут только при включённой настройке (гейт в плагине).
 */
import {
  initFrontendErrorCapture,
  logFront,
  setReportingEnabled,
  trackError as pluginTrackError,
  trackEvent as pluginTrackEvent,
} from "@my-tauri-plugins/plugin-logs";

let enabled = true;

/** Включена ли отправка телеметрии (по настройке пользователя). */
export function isTelemetryEnabled(): boolean {
  return enabled;
}

/** Принудительно включить/выключить отправку (при переключении галочки). */
export function setTelemetryEnabled(v: boolean): void {
  enabled = v;
  void setReportingEnabled(v);
}

/**
 * Инициализация при старте: вешает глобальные ловушки ошибок UI
 * (window error + unhandledrejection) и запускает регистрацию Web Component.
 */
export async function initTelemetry(): Promise<void> {
  initFrontendErrorCapture();
}

/**
 * Отправить событие-аналитику. Никогда не бросает и не ломает UI.
 * Возвращает false, если телеметрия выключена или не удалось отправить.
 */
export async function trackEvent(
  name: string,
  props?: Record<string, string | number>,
): Promise<boolean> {
  if (!enabled) return false;
  try {
    return await pluginTrackEvent(name, props as Record<string, unknown> | undefined);
  } catch {
    return false;
  }
}

/**
 * Отследить ОБРАБОТАННУЮ ошибку UI (catch в контроллерах).
 * Всегда пишет в локальный лог; в облако — только при включённой настройке.
 * Никогда не бросает и не ломает интерфейс.
 */
export async function trackError(whence: string, err: unknown): Promise<void> {
  const message = err instanceof Error ? err.message : String(err ?? "unknown error");
  const stack = err instanceof Error ? (err.stack || "") : "";
  logFront(`[UI Error] [${whence}] ${message}`);
  if (!enabled) return;
  try {
    await pluginTrackError({
      errorType: "UI Error",
      message: `[${whence}] ${message}`,
      stack: stack.slice(0, 2000),
    });
  } catch {
    /* сбой облачной отправки не должен ронять UI */
  }
}