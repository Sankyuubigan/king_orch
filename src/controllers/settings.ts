import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { store } from "../store";
import { bus } from "../events";
import { showToast } from "../ui";

export interface SettingsElements {
  modelSelect: HTMLSelectElement;
  agentSelect: HTMLSelectElement;
  contextSlider: HTMLInputElement;
  contextValue: HTMLElement;
  chkKvQuant: HTMLInputElement;
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
}

export class SettingsController {
  private el: SettingsElements;

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

  async loadConfig() {
    bus.emit("log", "Загрузка конфигурации...");
    try {
      const config: any = await invoke("get_config");
      const version: string = await invoke("get_app_version") as string;
      const verEl = document.getElementById("app-version");
      if (verEl) verEl.textContent = version;
      this.updateModelSelect(config);
      if (config.context_size) { this.el.contextSlider.value = config.context_size.toString(); this.el.contextValue.innerText = config.context_size.toString(); }
      if (config.kv_quantization !== undefined) this.el.chkKvQuant.checked = config.kv_quantization;
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
    this.el.contextSlider?.addEventListener("input", () => { this.el.contextValue.innerText = this.el.contextSlider.value; });
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
    this.el.btnAddModel?.addEventListener("click", async () => { try { const sel = await open({ filters: [{ name: "Model", extensions: ["gguf"] }] }); if (sel) { const cfg: any = await invoke("add_model", { path: sel as string }); this.updateModelSelect(cfg); await this.loadModelParams(); } } catch(e) {} });
    this.el.btnDownloadModel?.addEventListener("click", async () => {
      const name = this.el.downloadModelSelect.value; if (!name) return;
      const model = store.modelsCatalog.find(m => m.name === name); if (!model) return;
      try {
        const savePath = await save({ defaultPath: `${model.name}.gguf`, filters: [{ name: "GGUF", extensions: ["gguf"] }] }); if (!savePath) return;
        this.el.btnDownloadModel.disabled = true; this.el.downloadProgressContainer.style.display = "block";
        await invoke("download_model", { url: model.download_url, savePath }); await invoke("add_model", { path: savePath });
        await this.loadConfig(); showToast(`Модель ${model.name} скачана!`, "success");
      } catch(e) { showToast(`Ошибка: ${e}`, "error"); }
      finally { this.el.btnDownloadModel.disabled = false; this.el.downloadProgressContainer.style.display = "none"; }
    });
  }

  private bindTauriEvents() {
    listen("download_progress", (e: any) => { const { downloaded, total } = e.payload; const pct = total > 0 ? (downloaded / total) * 100 : 0; this.el.downloadProgressBar.style.width = `${pct}%`; this.el.downloadStatusLabel.innerText = `${(downloaded/1024/1024).toFixed(1)} MB / ${(total/1024/1024).toFixed(1)} MB`; });
  }
}