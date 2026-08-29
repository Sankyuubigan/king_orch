import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { invoke } from "@tauri-apps/api/core";
import { showToast } from "../ui";
import { trackError } from "../telemetry";

export interface UpdatePopupElements {
  container: HTMLElement;
  btnUpdate: HTMLButtonElement;
  btnLater: HTMLButtonElement;
}

interface GithubUpdateInfo {
  version: string;
  url: string;
  notes: string;
}

export class UpdatePopupController {
  private el: UpdatePopupElements;
  private pendingUpdate: Update | null = null;
  private pendingGithub: GithubUpdateInfo | null = null;

  constructor(el: UpdatePopupElements) {
    this.el = el;
    this.el.btnUpdate.addEventListener("click", () => this.install());
    this.el.btnLater.addEventListener("click", () => this.hide());
  }

  /** Проверка при старте приложения. Показывает попап, если есть новая версия. */
  async checkOnStartup() {
    this.hide();
    // 1. Основной путь: tauri-plugin-updater (raw.githubusercontent.com).
    try {
      const update = await check();
      if (update) {
        this.pendingUpdate = update;
        this.show();
        return;
      }
    } catch (e: any) {
      // Проверка при старте не должна мешать работе; идём в fallback.
      void trackError("updatePopup.checkOnStartup.plugin", e);
    }
    // 2. Резервный путь: GitHub Releases API (если raw.githubusercontent.com заблокирован).
    try {
      const info = await invoke<GithubUpdateInfo | null>("check_github_release_update");
      if (info) {
        this.pendingGithub = info;
        this.show();
      }
    } catch (e: any) {
      void trackError("updatePopup.checkOnStartup.github", e);
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
      // Резервный путь установки (GitHub Releases API).
      if (this.pendingGithub) {
        this.el.btnUpdate.innerText = "Скачивание...";
        await invoke("install_update_from_github", { url: this.pendingGithub.url });
        // Процесс завершится самим установщиком (exit(0)) — сюда не возвращаемся.
        return;
      }

      // Основной путь установки (tauri-plugin-updater).
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
