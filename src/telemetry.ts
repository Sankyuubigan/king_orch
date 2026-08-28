/**
 * 🛰️ Слой телеметрии фронтенда — единственное место, откуда уходят
 * ошибки UI в сервис сбора (сейчас Aptabase). Ошибки отправляются в
 * Error Reporting API (раздел «Errors» дашборда) через нашу Tauri-команду
 * `track_error` (бэкенд сам знает про App Key и endpoint).
 *
 * 🔄 Смена сервиса = переписать только бэкенд (infra/telemetry) и этот файл.
 *
 * Уважает настройку «Отправлять анонимные отчёты об ошибках»:
 * если юзер выключил — слушатели не вешаются, ошибки не отправляются,
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

  // Необработанная ошибка / краш UI → "Frontend Error" (kind=unhandled)
  window.addEventListener("error", (e) => {
    const message = e.message || (e.error instanceof Error ? e.error.message : "unknown error");
    const stack = e.error instanceof Error ? e.error.stack || "" : "";
    void sendError("Frontend Error", message, stack, "error", "unhandled");
  });

  // Необработанное отклонение промиса → "Frontend Error" (kind=unhandled)
  window.addEventListener("unhandledrejection", (e) => {
    const reason: unknown = e.reason;
    const message = reason instanceof Error ? reason.message : String(reason);
    const stack = reason instanceof Error ? reason.stack || "" : "";
    void sendError("Frontend Error", message, stack, "error", "unhandled");
  });
}

/**
 * Отправить событие-аналитику. Никогда не бросает и не ломает UI.
 * (Ошибки идут отдельно — через `sendError` в Error Reporting API.)
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
 * Отправить отчёт об ошибке в Error Reporting API.
 * Никогда не бросает и не ломает UI: сбой отправки только логируется.
 */
async function sendError(
  errorType: string,
  message: string,
  stack: string,
  severity: string,
  kind: string,
): Promise<void> {
  // ── Всегда пишем в ЛОКАЛЬНЫЙ лог (независимо от согласия на телеметрию) ──
  // иначе реальные ошибки фронта теряются и не видны в king_orch.log / last_logs.txt.
  try {
    await invoke("log_frontend_event", {
      level: "FE-ERR",
      msg: `[${errorType}] ${message}${stack ? ` | ${stack.slice(0, 1500)}` : ""}`,
    });
  } catch {
    /* локальная лог-команда недоступна — не ломаем UI */
  }

  if (!enabled) return;
  try {
    await invoke("track_error", {
      errorType,
      message,
      stack: stack || null,
      severity,
      kind,
    });
  } catch (err) {
    console.warn(`[telemetry] sendError("${errorType}") не удалось:`, err);
  }
}

/**
 * Отследить ОБРАБОТАННУЮ ошибку UI (catch в контроллерах).
 * Никогда не бросает и не ломает интерфейс.
 */
export async function trackError(whence: string, err: unknown): Promise<void> {
  const message = err instanceof Error ? err.message : String(err ?? "unknown error");
  const stack = err instanceof Error ? (err.stack || "") : "";
  await sendError("UI Error", `[${whence}] ${message}`, stack.slice(0, 2000), "error", "handled");
}