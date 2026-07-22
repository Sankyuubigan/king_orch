import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { check } from "@tauri-apps/plugin-updater";
import { store } from "../store";
import { bus } from "../events";
import { showToast } from "../ui";

export interface SettingsElements {
  modelSelect: HTMLSelectElement;
  agentSelect: HTMLSelectElement;
  contextSlider: HTMLInputElement; contextValue: HTMLElement;
  maxGenSlider: HTMLInputElement; maxGenValue: HTMLElement;
  chkKvQuantK: HTMLInputElement;
  chkKvQuantV: HTMLInputElement;
  themeSelect: HTMLSelectElement;
  promptFormatSelect: HTMLSelectElement;
  tempSlider: HTMLInputElement; tempValue: HTMLElement;
  topkSlider: HTMLInputElement; topkValue: HTMLElement;
  toppSlider: HTMLInputElement; toppValue: HTMLElement;
  minpSlider: HTMLInputElement; minpValue: HTMLElement;
  reppenSlider: HTMLInputElement; reppenValue: HTMLElement;
  prespenSlider: HTMLInputElement; prespenValue: HTMLElement;
  btnResetParams: HTMLButtonElement;
  downloadModelSelect: HTMLSelectElement;
  btnDownloadModel: HTMLButtonElement;
  downloadProgressContainer: HTMLDivElement;
  downloadProgressBar: HTMLDivElement;
  downloadStatusLabel: HTMLDivElement;
  btnAddModel: HTMLButtonElement;
  chkShowAdvanced: HTMLInputElement;
  modelsList: HTMLDivElement;
  btnAddModelLlm: HTMLButtonElement;
  btnCheckUpdate: HTMLButtonElement;
  btnInstallUpdate: HTMLButtonElement;
  updateStatus: HTMLElement;
  btnAutoDownload: HTMLButtonElement;
  autoDownloadModal: HTMLElement;
  modalModelName: HTMLElement;
  modalSavePath: HTMLElement;
  modalFreeSpace: HTMLElement;
  btnModalCancel: HTMLButtonElement;
  btnModalConfirm: HTMLButtonElement;
}

export class SettingsController {
  private el: SettingsElements;
  private pendingUpdate: any = null;

  constructor(el: SettingsElements) {
    this.el = el;
    this.bindDomEvents();
    this.bindTauriEvents();
  }

  async loadModelParams() {
    const p = this.el.modelSelect.value; if (!p) return;
    const params: any = await invoke("get_model_params", { modelPath: p });
    this.el.tempSlider.value = params.temperature; this.el.tempValue.innerText = params.temperature;
    this.el.topkSlider.value = params.top_k; this.el.topkValue.innerText = params.top_k;
    this.el.toppSlider.value = params.top_p; this.el.toppValue.innerText = params.top_p;
    this.el.minpSlider.value = params.min_p; this.el.minpValue.innerText = params.min_p;
    this.el.reppenSlider.value = params.repetition_penalty; this.el.reppenValue.innerText = params.repetition_penalty;
    this.el.prespenSlider.value = params.presence_penalty; this.el.prespenValue.innerText = params.presence_penalty;
  }

  private async saveModelParams() {
    const p = this.el.modelSelect.value; if (!p) return;
    await invoke("set_model_params", { modelPath: p, params: { temperature: parseFloat(this.el.tempSlider.value), top_k: parseInt(this.el.topkSlider.value, 10), top_p: parseFloat(this.el.toppSlider.value), min_p: parseFloat(this.el.minpSlider.value), repetition_penalty: parseFloat(this.el.reppenSlider.value), presence_penalty: parseFloat(this.el.prespenSlider.value) } });
  }

  updateModelSelect(config: any) {
    this.el.modelSelect.innerHTML = "";
    for (const m of config.models) { const o = document.createElement("option"); o.value = m; o.text = m.split(/[/\\]/).pop() || m; this.el.modelSelect.appendChild(o); }
    if (config.last_model && config.models.includes(config.last_model)) this.el.modelSelect.value = config.last_model;
  }

  renderModelsList(config: any) {
    this.el.modelsList.innerHTML = "";
    if (!config.models || config.models.length === 0) {
      const empty = document.createElement("div");
      empty.className = "models-list-empty";
      empty.style.color = "var(--text-muted, #888)";
      empty.style.fontSize = "13px";
      empty.innerText = "Модели не добавлены.";
      this.el.modelsList.appendChild(empty);
      this.el.btnAutoDownload.style.display = "block";
      return;
    }
    this.el.btnAutoDownload.style.display = "none";
    for (const m of config.models) {
      const row = document.createElement("div");
      row.className = "model-list-row";
      row.style.cssText = "display:flex; align-items:center; justify-content:space-between; gap:10px; padding:8px 10px; border:1px solid var(--border, #333); border-radius:6px; background:var(--bg-elevated, #1c1c1c);";

      const info = document.createElement("div");
      info.style.cssText = "display:flex; flex-direction:column; gap:2px; min-width:0;";
      const name = document.createElement("div");
      const fileName = m.split(/[/\\]/).pop() || m;
      name.innerText = (config.last_model === m ? "● " : "") + fileName;
      name.style.cssText = "font-weight:600; color:var(--text, #eee); word-break:break-all;";
      const path = document.createElement("div");
      path.innerText = m;
      path.style.cssText = "font-size:11px; color:var(--text-muted, #888); word-break:break-all;";
      info.appendChild(name);
      info.appendChild(path);

      const btnRemove = document.createElement("button");
      btnRemove.className = "btn-danger";
      btnRemove.innerText = "Удалить";
      btnRemove.style.cssText = "flex-shrink:0; padding:4px 12px;";
      btnRemove.addEventListener("click", async () => {
        if (!confirm(`Удалить модель «${fileName}» из списка? Файл на диске не будет удалён.`)) return;
        try {
          const cfg: any = await invoke("remove_model", { path: m });
          this.updateModelSelect(cfg);
          this.renderModelsList(cfg);
          showToast("Модель удалена из списка.", "success");
        } catch (e) { showToast(`Ошибка: ${e}`, "error"); }
      });

      row.appendChild(info);
      row.appendChild(btnRemove);
      this.el.modelsList.appendChild(row);
    }
  }

  async loadConfig() {
    bus.emit("log", "Загрузка конфигурации...");
    try {
      const config: any = await invoke("get_config");
      const version: string = await invoke("get_app_version") as string;
      const verEl = document.getElementById("app-version");
      if (verEl) verEl.textContent = version;
      if (this.el.updateStatus) this.el.updateStatus.textContent = "";
      this.updateModelSelect(config);
      this.renderModelsList(config);
      if (config.context_size) { this.el.contextSlider.value = config.context_size.toString(); this.el.contextValue.innerText = config.context_size.toString(); }
      if (config.max_gen_tokens) { this.el.maxGenSlider.value = config.max_gen_tokens.toString(); this.el.maxGenValue.innerText = config.max_gen_tokens.toString(); }
      if (config.kv_quant_keys !== undefined) this.el.chkKvQuantK.checked = config.kv_quant_keys;
      if (config.kv_quant_values !== undefined) this.el.chkKvQuantV.checked = config.kv_quant_values;
      if (config.theme) { this.el.themeSelect.value = config.theme; document.documentElement.setAttribute('data-theme', config.theme); }
      if (config.prompt_format) this.el.promptFormatSelect.value = config.prompt_format;
      if (config.show_advanced_features !== undefined) {
        this.el.chkShowAdvanced.checked = config.show_advanced_features;
        store.showAdvancedFeatures = config.show_advanced_features;
        bus.emit("advanced:visibility", config.show_advanced_features);
      }
      await this.loadAgents();
      bus.emit("config:loaded", config);
      await this.loadCatalog();
      await this.loadModelParams();
    } catch(e) { showToast(`Ошибка: ${e}`, "error"); }
  }

  private async loadAgents() {
    try {
      const entries: any[] = await invoke("get_agents");
      this.el.agentSelect.innerHTML = '';
      for (const e of entries) {
        if (!e.is_hidden) {
          const o = document.createElement("option");
          o.value = e.id;
          const prefix = e.entry_type === 'workflow' ? '📁' : '📊';
          const folderPart = e.folder ? `${e.folder} - ` : '';
          o.text = `${prefix} ${folderPart}${e.name} (${e.id})`;
          this.el.agentSelect.appendChild(o);
        }
      }
    } catch(e) {}
  }

  private async loadCatalog() {
    try {
      store.modelsCatalog = await invoke("get_models_catalog");
      this.el.downloadModelSelect.innerHTML = '<option value="">-- Выберите модель --</option>';
      store.modelsCatalog.forEach(m => { const o = document.createElement("option"); o.value = m.name; o.text = m.name; this.el.downloadModelSelect.appendChild(o); });
    } catch(e) { this.el.downloadModelSelect.innerHTML = '<option value="">Ошибка</option>'; }
  }

  private bindDomEvents() {
    this.el.contextSlider?.addEventListener("input", async () => { 
        this.el.contextValue.innerText = this.el.contextSlider.value; 
        await invoke("set_config_value", { key: "context_size", value: parseInt(this.el.contextSlider.value, 10) });
    });
    this.el.maxGenSlider?.addEventListener("input", async () => { 
        this.el.maxGenValue.innerText = this.el.maxGenSlider.value; 
        await invoke("set_config_value", { key: "max_gen_tokens", value: parseInt(this.el.maxGenSlider.value, 10) });
    });
    this.el.chkKvQuantK?.addEventListener("change", async () => {
        await invoke("set_config_value", { key: "kv_quant_keys", value: this.el.chkKvQuantK.checked });
    });
    this.el.chkKvQuantV?.addEventListener("change", async () => {
        await invoke("set_config_value", { key: "kv_quant_values", value: this.el.chkKvQuantV.checked });
    });
    this.el.themeSelect?.addEventListener("change", async () => { document.documentElement.setAttribute('data-theme', this.el.themeSelect.value); await invoke("set_theme", { theme: this.el.themeSelect.value }); });
    this.el.promptFormatSelect?.addEventListener("change", async () => { await invoke("set_prompt_format", { format: this.el.promptFormatSelect.value }); });
    const sliders: [HTMLInputElement, HTMLElement][] = [[this.el.tempSlider, this.el.tempValue],[this.el.topkSlider, this.el.topkValue],[this.el.toppSlider, this.el.toppValue],[this.el.minpSlider, this.el.minpValue],[this.el.reppenSlider, this.el.reppenValue],[this.el.prespenSlider, this.el.prespenValue]];
    for (const [s, l] of sliders) s?.addEventListener("input", () => { l.innerText = s.value; this.saveModelParams(); });
    this.el.btnResetParams?.addEventListener("click", async () => { const p = this.el.modelSelect.value; if (!p) return; await invoke("reset_model_params", { modelPath: p }); await this.loadModelParams(); showToast("Параметры сброшены.", "success"); });
    this.el.modelSelect?.addEventListener("change", async () => { await invoke("set_last_model", { path: this.el.modelSelect.value }); await this.loadModelParams(); });
    this.el.chkShowAdvanced?.addEventListener("change", async () => {
      const val = this.el.chkShowAdvanced.checked;
      store.showAdvancedFeatures = val;
      await invoke("set_config_value", { key: "show_advanced_features", value: val });
      bus.emit("advanced:visibility", val);
    });
    this.el.btnAddModel?.addEventListener("click", async () => { try { const sel = await open({ filters: [{ name: "Model", extensions: ["gguf"] }] }); if (sel) { const cfg: any = await invoke("add_model", { path: sel as string }); this.updateModelSelect(cfg); this.renderModelsList(cfg); await this.loadModelParams(); } } catch(e) { showToast(`Не удалось добавить модель: ${e}`, "error"); } });
    this.el.btnAddModelLlm?.addEventListener("click", async () => { try { const sel = await open({ filters: [{ name: "Model", extensions: ["gguf"] }] }); if (sel) { const cfg: any = await invoke("add_model", { path: sel as string }); this.updateModelSelect(cfg); this.renderModelsList(cfg); await this.loadModelParams(); } } catch(e) { showToast(`Не удалось добавить модель: ${e}`, "error"); } });
    this.el.btnCheckUpdate?.addEventListener("click", async () => {
      const btn = this.el.btnCheckUpdate;
      const status = this.el.updateStatus;
      btn.disabled = true;
      status.textContent = "Проверка...";
      this.el.btnInstallUpdate.style.display = "none";
      this.pendingUpdate = null;
      try {
        const update = await check();
        if (update) {
          status.textContent = `Доступна версия ${update.version}`;
          this.el.btnInstallUpdate.style.display = "inline-block";
          this.pendingUpdate = update;
        } else {
          status.textContent = "У вас актуальная версия";
        }
      } catch (e: any) {
        status.textContent = "";
        showToast(`Ошибка проверки обновлений: ${e}`, "error");
      } finally {
        btn.disabled = false;
      }
    });
    this.el.btnInstallUpdate?.addEventListener("click", async () => {
      if (!this.pendingUpdate) return;
      const btn = this.el.btnInstallUpdate;
      const status = this.el.updateStatus;
      btn.disabled = true;
      this.el.btnCheckUpdate.disabled = true;
      status.textContent = "Установка...";
      try {
        await this.pendingUpdate.downloadAndInstall();
        status.textContent = "Обновление установлено. Перезапустите приложение.";
        this.el.btnInstallUpdate.style.display = "none";
        this.pendingUpdate = null;
      } catch (e: any) {
        status.textContent = "";
        showToast(`Ошибка установки: ${e}`, "error");
      } finally {
        btn.disabled = false;
        this.el.btnCheckUpdate.disabled = false;
      }
    });
    this.el.btnDownloadModel?.addEventListener("click", async () => {
      const name = this.el.downloadModelSelect.value; if (!name) return;
      const model = store.modelsCatalog.find(m => m.name === name); if (!model) return;
      try {
         const savePath = await save({ defaultPath: model.download_url.split('/').pop()?.split('?')[0] || `${model.name}.gguf`, filters: [{ name: "GGUF", extensions: ["gguf"] }] }); if (!savePath) return;
        this.el.btnDownloadModel.disabled = true; this.el.downloadProgressContainer.style.display = "block";
        await invoke("download_model", { url: model.download_url, savePath }); await invoke("add_model", { path: savePath });
        await this.loadConfig(); showToast(`Модель ${model.name} скачана!`, "success");
      } catch(e) { showToast(`Ошибка: ${e}`, "error"); }
      finally { this.el.btnDownloadModel.disabled = false; this.el.downloadProgressContainer.style.display = "none"; }
    });

    this.el.btnAutoDownload?.addEventListener("click", async () => {
      try {
        const info: any = await invoke("get_auto_download_info");
        this.el.modalModelName.innerText = info.model_name;
        this.el.modalSavePath.innerText = info.save_path;
        this.el.modalFreeSpace.innerText = `${info.free_space_gb} GB`;
        this.el.autoDownloadModal.style.display = "flex";

        const confirmed = await new Promise<boolean>((resolve) => {
          this.el.btnModalConfirm.onclick = () => { resolve(true); };
          this.el.btnModalCancel.onclick = () => { resolve(false); };
        });
        this.el.autoDownloadModal.style.display = "none";
        if (!confirmed) return;

        this.el.btnDownloadModel.disabled = true;
        this.el.downloadProgressContainer.style.display = "block";
        await invoke("auto_download_default_model", { savePath: info.save_path });
        await this.loadConfig();
        showToast(`Модель ${info.model_name} скачана!`, "success");
      } catch(e) { showToast(`Ошибка: ${e}`, "error"); }
      finally { this.el.btnDownloadModel.disabled = false; this.el.downloadProgressContainer.style.display = "none"; }
    });
  }

  private bindTauriEvents() {
    listen("download_progress", (e: any) => { const { downloaded, total } = e.payload; const pct = total > 0 ? (downloaded / total) * 100 : 0; this.el.downloadProgressBar.style.width = `${pct}%`; this.el.downloadStatusLabel.innerText = `${(downloaded/1024/1024).toFixed(1)} MB / ${(total/1024/1024).toFixed(1)} MB`; });
  }
}