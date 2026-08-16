import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { showToast } from "../ui";
import { trackError } from "../telemetry";

export interface UpdatePopupElements {
  container: HTMLElement;
  btnUpdate: HTMLButtonElement;
  btnLater: HTMLButtonElement;
}

export class UpdatePopupController {
  private el: UpdatePopupElements;
  private pendingUpdate: Update | null = null;

  constructor(el: UpdatePopupElements) {
    this.el = el;
    this.el.btnUpdate.addEventListener("click", () => this.install());
    this.el.btnLater.addEventListener("click", () => this.hide());
  }

  /** Проверка при старте приложения. Показывает попап, если есть новая версия. */
  async checkOnStartup() {
    this.hide();
    try {
      const update = await check();
      if (update) {
        this.pendingUpdate = update;
        this.show();
      }
    } catch (e: any) {
      // Проверка при старте не должна мешать работе приложения.
      void trackError("updatePopup.checkOnStartup", e);
    }
  }

  private show() {
    this.el.container.style.display = "block";
  }

  private hide() {
    this.el.container.style.display = "none";
  }

  private async install() {
    this.el.btnUpdate.disabled = true;
    this.el.btnLater.disabled = true;
    try {
      // Повторно берём самую свежую версию (могла выйти ещё одна за это время).
      const update = (await check()) ?? this.pendingUpdate;
      if (!update) {
        showToast("Обновление не найдено.", "info");
        this.hide();
        return;
      }
      this.pendingUpdate = update;
      this.el.btnUpdate.innerText = "Установка...";
      await update.downloadAndInstall();
      this.el.btnUpdate.innerText = "Перезапуск...";
      await relaunch();
    } catch (e: any) {
      showToast(`Ошибка обновления: ${e}`, "error");
      void trackError("updatePopup.install", e);
      this.el.btnUpdate.disabled = false;
      this.el.btnLater.disabled = false;
      this.el.btnUpdate.innerText = "Обновить и перезапустить";
    }
  }
}
