import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { store } from "../store";
import { bus } from "../events";
import { showToast } from "../ui";
import { setTelemetryEnabled, trackError } from "../telemetry";
import { formatBytes, formatSpeed } from "../utils";

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
  chkShowFolderAgents: HTMLInputElement;
  chkErrorReports: HTMLInputElement;
  modelsList: HTMLDivElement;
  btnAddModelLlm: HTMLButtonElement;
  translatorModelSelect: HTMLSelectElement;
  translatorLangSelect: HTMLSelectElement;
  btnAutoDownload: HTMLButtonElement;
  autoDownloadModal: HTMLElement;
  modalModelName: HTMLElement;
  modalSavePath: HTMLElement;
  modalFreeSpace: HTMLElement;
  btnModalCancel: HTMLButtonElement;
  btnModalConfirm: HTMLButtonElement;
  engineStatus: HTMLElement;
  engineGpu: HTMLElement;
  enginePath: HTMLElement;
  engineVariantSelect: HTMLSelectElement;
  engineVariantHint: HTMLElement;
  btnApplyEngineVariant: HTMLButtonElement;
  btnInstallEngine: HTMLButtonElement;
  btnCheckEngineUpdate: HTMLButtonElement;
  btnInstallEngineUpdate: HTMLButtonElement;
  btnRemoveEngine: HTMLButtonElement;
  btnSetEngineDir: HTMLButtonElement;
  engineProgressContainer: HTMLDivElement;
  engineProgressBar: HTMLDivElement;
  engineStatusLabel: HTMLDivElement;
  engineWarning: HTMLElement;
  chatFontSlider: HTMLInputElement;
  chatFontValue: HTMLElement;
}

export class SettingsController {
  private el: SettingsElements;
  /// Когда в последний раз приходил чанк при скачивании (для детекта «зависло»).
  private lastDownloadActivity = 0;

  constructor(el: SettingsElements) {
    this.el = el;
    this.bindDomEvents();
    this.bindTauriEvents();
    this.bindBusEvents();
    this.watchDownloadStall();
  }

  /// Если качает, но прогресс/скорость не приходят дольше 1.5с — подсвечиваем
  /// пульсацией, чтобы не выглядело, будто всё зависло намертво.
  private watchDownloadStall() {
    window.setInterval(() => {
      const container = this.el.downloadProgressContainer;
      if (!container || container.style.display === "none") return;
      const stalled = this.lastDownloadActivity > 0 && Date.now() - this.lastDownloadActivity > 1500;
      container.classList.toggle("download-stalled", stalled);
    }, 500);
  }

  async loadModelParams() {
    const p = this.el.modelSelect.value; if (!p) return;
    const params: any = await invoke("get_model_params", { modelPath: p });
    store.currentModelParams = params;
    this.el.tempSlider.value = params.temperature; this.el.tempValue.innerText = params.temperature;
    this.el.topkSlider.value = params.top_k; this.el.topkValue.innerText = params.top_k;
    this.el.toppSlider.value = params.top_p; this.el.toppValue.innerText = params.top_p;
    this.el.minpSlider.value = params.min_p; this.el.minpValue.innerText = params.min_p;
    this.el.reppenSlider.value = params.repetition_penalty; this.el.reppenValue.innerText = params.repetition_penalty;
    this.el.prespenSlider.value = params.presence_penalty; this.el.prespenValue.innerText = params.presence_penalty;
  }

  private async saveModelParams() {
    const p = this.el.modelSelect.value; if (!p) return;
    const base = store.currentModelParams;
    await invoke("set_model_params", { modelPath: p, params: {
      temperature: parseFloat(this.el.tempSlider.value),
      top_k: parseInt(this.el.topkSlider.value, 10),
      top_p: parseFloat(this.el.toppSlider.value),
      min_p: parseFloat(this.el.minpSlider.value),
      repetition_penalty: parseFloat(this.el.reppenSlider.value),
      presence_penalty: parseFloat(this.el.prespenSlider.value),
      dry_multiplier: base?.dry_multiplier ?? 0.0,
      dry_base: base?.dry_base ?? 1.75,
      dry_allowed_length: base?.dry_allowed_length ?? 2,
      dry_penalty_last_n: base?.dry_penalty_last_n ?? 0,
      xtc_probability: base?.xtc_probability ?? 0.0,
      xtc_threshold: base?.xtc_threshold ?? 0.1,
    } });
  }

  private capMap: Record<string, { uncen: boolean; vision: boolean; audio: boolean }> = {};

  /// Загружает актуальные возможности всех моделей (живой
  /// `get_all_capabilities`, единый источник правды) и перерисовывает
  /// списки моделей. Заменяет использование устаревающего `model_meta`
  /// для отображения иконок (см. баг с нотой 🎵 у Gemma).
  private async refreshModelLists(config: any) {
    try {
      this.capMap = await invoke<Record<string, { uncen: boolean; vision: boolean; audio: boolean }>>("get_all_capabilities");
    } catch (_) { this.capMap = {}; }
    this.updateModelSelect(config);
    this.renderModelsList(config);
  }

  updateModelSelect(config: any) {
    this.el.modelSelect.innerHTML = "";
    for (const m of config.models) {
      const o = document.createElement("option");
      o.value = m;
      const fileName = m.split(/[/\\]/).pop() || m;
      const badges = this.capabilityIcons(this.capMap[m]).map(i => i.icon).join(" ");
      o.text = fileName + (badges ? `  ${badges}` : "");
      this.el.modelSelect.appendChild(o);
    }
    if (config.last_model && config.models.includes(config.last_model)) this.el.modelSelect.value = config.last_model;
  }

  /// Заполняет выпадающий список модели-переводчика списком установленных моделей.
  updateTranslatorModelSelect(config: any) {
    const sel = this.el.translatorModelSelect;
    if (!sel) return;
    sel.innerHTML = '<option value="">— не выбрано —</option>';
    for (const m of config.models || []) {
      const o = document.createElement("option");
      o.value = m;
      o.text = m.split(/[/\\]/).pop() || m;
      sel.appendChild(o);
    }
    if (config.translator_model && config.models && config.models.includes(config.translator_model)) {
      sel.value = config.translator_model;
    }
  }

  private capabilityIcons(meta?: { uncen?: boolean; vision?: boolean; audio?: boolean }): { icon: string; title: string }[] {
    const out: { icon: string; title: string }[] = [];
    if (meta?.uncen) out.push({ icon: "😈", title: "Без цензуры (uncensored)" });
    if (meta?.vision) out.push({ icon: "👁️", title: "Видит изображения (vision)" });
    if (meta?.audio) out.push({ icon: "🎵", title: "Понимает аудио (audio)" });
    return out;
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
      const meta = this.capMap[m];
      for (const b of this.capabilityIcons(meta)) {
        const span = document.createElement("span");
        span.innerText = b.icon;
        span.title = b.title;
        span.style.cssText = "font-size:13px; margin-left:5px; cursor:help;";
        name.appendChild(span);
      }
      info.appendChild(name);
      info.appendChild(path);

      const btnRemove = document.createElement("button");
      btnRemove.className = "btn-secondary";
      btnRemove.innerText = "Удалить из списка";
      btnRemove.style.cssText = "flex-shrink:0; padding:4px 12px;";
      btnRemove.addEventListener("click", async () => {
        if (!confirm(`Удалить модель «${fileName}» из списка? Файл на диске не будет удалён.`)) return;
        try {
          const cfg: any = await invoke("remove_model", { path: m });
          await this.refreshModelLists(cfg);
          showToast("Модель удалена из списка.", "success");
        } catch (e) { showToast(`Ошибка: ${e}`, "error"); void trackError("settings.removeModel", e); }
      });

      const btnDeleteFile = document.createElement("button");
      btnDeleteFile.className = "btn-danger";
      btnDeleteFile.innerText = "Удалить файл";
      btnDeleteFile.style.cssText = "flex-shrink:0; padding:4px 12px;";
      btnDeleteFile.addEventListener("click", async () => {
        if (!confirm(`Удалить файл модели «${fileName}» с диска БЕЗВОЗВРАТНО? Запись тоже исчезнет из списка.`)) return;
        try {
          const cfg: any = await invoke("delete_model_file", { path: m });
          await this.refreshModelLists(cfg);
          showToast("Файл модели удалён.", "success");
        } catch (e) { showToast(`Ошибка: ${e}`, "error"); void trackError("settings.deleteModelFile", e); }
      });

      const btnGroup = document.createElement("div");
      btnGroup.style.cssText = "display:flex; gap:8px; flex-shrink:0;";
      btnGroup.appendChild(btnRemove);
      btnGroup.appendChild(btnDeleteFile);

      row.appendChild(info);
      row.appendChild(btnGroup);
      this.el.modelsList.appendChild(row);
    }
  }

  async loadConfig() {
    bus.emit("log", "Загрузка конфигурации...");
    try {
      const config: any = await invoke("get_config");
      await this.refreshModelLists(config);
      this.updateTranslatorModelSelect(config);
      if (config.translator_model) {
        this.el.translatorModelSelect.value = config.translator_model;
        store.translatorModel = config.translator_model;
      } else {
        store.translatorModel = null;
      }
      if (config.translator_lang) {
        this.el.translatorLangSelect.value = config.translator_lang;
        store.translatorLang = config.translator_lang;
      }
      if (config.context_size) { this.el.contextSlider.value = config.context_size.toString(); this.el.contextValue.innerText = config.context_size.toString(); }
      if (config.max_gen_tokens) { this.el.maxGenSlider.value = config.max_gen_tokens.toString(); this.el.maxGenValue.innerText = config.max_gen_tokens.toString(); }
      if (config.chat_font_scale !== undefined) {
        const pct = Math.round(config.chat_font_scale * 100);
        this.el.chatFontSlider.value = pct.toString();
        this.el.chatFontValue.innerText = `${pct}%`;
        this.applyChatFontScale(config.chat_font_scale);
      }
      if (config.kv_quant_keys !== undefined) this.el.chkKvQuantK.checked = config.kv_quant_keys;
      if (config.kv_quant_values !== undefined) this.el.chkKvQuantV.checked = config.kv_quant_values;
      if (config.theme) { this.el.themeSelect.value = config.theme; document.documentElement.setAttribute('data-theme', config.theme); }
      if (config.prompt_format) this.el.promptFormatSelect.value = config.prompt_format;
      if (config.show_advanced_features !== undefined) {
        this.el.chkShowAdvanced.checked = config.show_advanced_features;
        store.showAdvancedFeatures = config.show_advanced_features;
        bus.emit("advanced:visibility", config.show_advanced_features);
      }
      if (config.show_folder_agents !== undefined) {
        this.el.chkShowFolderAgents.checked = config.show_folder_agents;
        store.showFolderAgents = config.show_folder_agents;
      }
      if (config.allow_error_reports !== undefined) {
        this.el.chkErrorReports.checked = config.allow_error_reports;
        setTelemetryEnabled(config.allow_error_reports);
      }
      await this.loadAgents(config.last_agent);
      bus.emit("config:loaded", config);
      await this.loadCatalog();
      await this.loadModelParams();
      await this.refreshEngineStatus();
    } catch(e) { showToast(`Ошибка: ${e}`, "error"); void trackError("settings.loadConfig", e); }
  }

  /** Применяет масштаб шрифта чата (1.0 = 100%) через CSS-переменную. */
  private applyChatFontScale(scale: number) {
    document.documentElement.style.setProperty("--chat-font-scale", String(scale));
  }

  async refreshEngineStatus() {
    try {
      const st: any = await invoke("get_engine_status");
      this.el.engineStatus.textContent = st.message;
      this.el.engineGpu.textContent = st.has_nvidia
        ? `${st.gpu_name} (драйвер CUDA ${st.cuda_major}.${st.cuda_minor}${st.compute_cap ? `, compute ${st.compute_cap}` : ""}; рекомендуемый вариант: ${st.required_variant || "?"})`
        : "Не обнаружена";
      this.el.enginePath.textContent = st.path || "—";

      // ── Дропдаун выбора бекенда ──
      const sel = this.el.engineVariantSelect;
      const prev = sel.value;
      sel.innerHTML = "";
      const autoOpt = document.createElement("option");
      autoOpt.value = "auto";
      autoOpt.text = "Авто (рекомендуется)";
      sel.appendChild(autoOpt);
      for (const v of st.available_variants || []) {
        const o = document.createElement("option");
        o.value = v.id;
        o.text = v.installed
          ? `${v.label} — установлен`
          : v.recommended
            ? `${v.label} (рекомендуется)`
            : v.label;
        sel.appendChild(o);
      }
      sel.value = st.selected_variant || "auto";
      sel.dataset.applied = sel.value;
      this.updateEngineVariantHint(st, sel.value);
      // Кнопка «Применить» — только если выбранное отличается от текущего
      this.el.btnApplyEngineVariant.style.display =
        sel.value === (st.selected_variant || "auto") ? "none" : "inline-block";

      if (st.installed) {
        this.el.btnInstallEngine.style.display = "none";
        this.el.btnCheckEngineUpdate.style.display = "inline-block";
        this.el.btnRemoveEngine.style.display = "inline-block";
        this.el.btnSetEngineDir.style.display = "inline-block";
        this.el.engineWarning.style.display = "none";
      } else {
        this.el.btnInstallEngine.style.display = "inline-block";
        this.el.btnCheckEngineUpdate.style.display = "none";
        this.el.btnRemoveEngine.style.display = "none";
        this.el.btnSetEngineDir.style.display = "inline-block";
        this.el.btnInstallEngineUpdate.style.display = "none";
        if (st.requires_driver_update) {
          this.el.btnInstallEngine.disabled = true;
          this.el.engineWarning.style.display = "block";
          this.el.engineWarning.textContent =
            `⚠️ Ваш драйвер NVIDIA поддерживает только CUDA ${st.cuda_major}.${st.cuda_minor}.\n` +
            `Для GPU-ускорения обновите драйвер (нужна версия ≥ 527.41, CUDA 12+).\n` +
            `Пока приложение работает в CPU-режиме.`;
        } else if (!st.has_nvidia) {
          this.el.btnInstallEngine.disabled = false;
          this.el.engineWarning.style.display = "none";
        } else {
          this.el.btnInstallEngine.disabled = false;
          this.el.engineWarning.style.display = "none";
        }
      }
    } catch (e) { showToast(`Ошибка статуса движка: ${e}`, "error"); void trackError("settings.engineStatus", e); }
  }

  /** Подсказка под дропдауном: что выбрано, что установлено */
  private updateEngineVariantHint(st: any, value: string) {
    const hint = this.el.engineVariantHint;
    if (!hint) return;
    const lines: string[] = [];
    if (value === "auto") {
      const resolved = (st.available_variants || []).find((v: any) => v.id === st.resolved_variant);
      lines.push(`Авто-подбор для этой машины: ${resolved ? resolved.label : (st.resolved_variant || "—")}.`);
    } else {
      const v = (st.available_variants || []).find((x: any) => x.id === value);
      if (v && v.note) lines.push(v.note);
    }
    const installed = st.installed_variants || [];
    if (installed.length > 0) {
      const names = (st.available_variants || [])
        .filter((x: any) => installed.includes(x.id))
        .map((x: any) => x.label)
        .join(", ");
      lines.push(`Установлены: ${names || installed.join(", ")}.`);
    } else {
      lines.push("Ни один бекенд ещё не установлен.");
    }
    lines.push("Смена бекенда применится при следующем запуске модели.");
    hint.textContent = lines.join("\n");
  }

  private async loadAgents(lastAgent?: string) {
    try {
      const entries: any[] = await invoke("get_agents");
      this.el.agentSelect.innerHTML = '';
      for (const e of entries) {
        if (!e.is_hidden && (e.folder === null || store.showFolderAgents)) {
          const o = document.createElement("option");
          o.value = e.id;
          const prefix = e.entry_type === 'workflow' ? '📁' : '📊';
          const folderPart = e.folder ? `${e.folder} - ` : '';
          o.text = `${prefix} ${folderPart}${e.name} (${e.id})`;
          this.el.agentSelect.appendChild(o);
        }
      }
      if (lastAgent && Array.from(this.el.agentSelect.options).some(o => o.value === lastAgent)) {
        this.el.agentSelect.value = lastAgent;
      }
    } catch(e) { void trackError("settings.loadAgents", e); }
  }

  private async loadCatalog() {
    try {
      store.modelsCatalog = await invoke("get_models_catalog");
      this.el.downloadModelSelect.innerHTML = '<option value="">-- Выберите модель --</option>';
      store.modelsCatalog.forEach(m => { const o = document.createElement("option"); o.value = m.name; const badges = this.capabilityIcons(m).map(i => i.icon).join(" "); o.text = (m.size_gb ? `${m.name} (${m.size_gb} GB)` : m.name) + (badges ? `  ${badges}` : ""); this.el.downloadModelSelect.appendChild(o); });
    } catch(e) { this.el.downloadModelSelect.innerHTML = '<option value="">Ошибка</option>'; void trackError("settings.loadCatalog", e); }
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
    this.el.chatFontSlider?.addEventListener("input", async () => {
        const pct = parseInt(this.el.chatFontSlider.value, 10);
        const scale = pct / 100;
        this.el.chatFontValue.innerText = `${pct}%`;
        this.applyChatFontScale(scale);
        await invoke("set_config_value", { key: "chat_font_scale", value: scale });
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
    this.el.agentSelect?.addEventListener("change", async () => { await invoke("set_config_value", { key: "last_agent", value: this.el.agentSelect.value }); });
    this.el.translatorModelSelect?.addEventListener("change", async () => {
      const v = this.el.translatorModelSelect.value || "";
      store.translatorModel = v || null;
      await invoke("set_config_value", { key: "translator_model", value: v });
    });
    this.el.translatorLangSelect?.addEventListener("change", async () => {
      const v = this.el.translatorLangSelect.value;
      store.translatorLang = v;
      await invoke("set_config_value", { key: "translator_lang", value: v });
    });
    this.el.chkShowAdvanced?.addEventListener("change", async () => {
      const val = this.el.chkShowAdvanced.checked;
      store.showAdvancedFeatures = val;
      await invoke("set_config_value", { key: "show_advanced_features", value: val });
      bus.emit("advanced:visibility", val);
    });
    this.el.chkShowFolderAgents?.addEventListener("change", async () => {
      const val = this.el.chkShowFolderAgents.checked;
      store.showFolderAgents = val;
      await invoke("set_config_value", { key: "show_folder_agents", value: val });
      await this.loadAgents();
    });
    this.el.chkErrorReports?.addEventListener("change", async () => {
      const val = this.el.chkErrorReports.checked;
      // Мгновенно синхронизируем состояние телеметрии и сохраняем настройку.
      setTelemetryEnabled(val);
      await invoke("set_config_value", { key: "allow_error_reports", value: val });
    });
    this.el.btnAddModel?.addEventListener("click", async () => { try { const sel = await openDialog({ filters: [{ name: "Model", extensions: ["gguf"] }] }); if (sel) {         const res: any = await invoke("add_model", { path: sel as string, flags: null }); if (res?.warning) showToast(res.warning, "error"); await this.refreshModelLists(res.config); await this.loadModelParams(); } } catch(e) { showToast(`Не удалось добавить модель: ${e}`, "error"); void trackError("settings.addModel", e); } });
    this.el.btnAddModelLlm?.addEventListener("click", async () => { try { const sel = await openDialog({ filters: [{ name: "Model", extensions: ["gguf"] }] }); if (sel) {         const res: any = await invoke("add_model", { path: sel as string, flags: null }); if (res?.warning) showToast(res.warning, "error"); await this.refreshModelLists(res.config); await this.loadModelParams(); } } catch(e) { showToast(`Не удалось добавить модель: ${e}`, "error"); void trackError("settings.addModel", e); } });
    this.el.btnDownloadModel?.addEventListener("click", async () => {
      const name = this.el.downloadModelSelect.value; if (!name) return;
      const model = store.modelsCatalog.find(m => m.name === name); if (!model) return;
      try {
         const savePath = await save({ defaultPath: model.download_url.split('/').pop()?.split('?')[0] || `${model.name}.gguf`, filters: [{ name: "GGUF", extensions: ["gguf"] }] }); if (!savePath) return;
        this.el.btnDownloadModel.disabled = true; this.resetDownloadUi();
        await invoke("download_model", { url: model.download_url, savePath });
        if (model.mmproj_url) {
          const sep = savePath.includes("\\") ? "\\" : "/";
          const dir = savePath.substring(0, savePath.lastIndexOf(sep));
          const mpName = (model.mmproj_url.split('/').pop() || "mmproj.gguf").split('?')[0];
          const mpPath = `${dir}${sep}${mpName}`;
          await invoke("download_model", { url: model.mmproj_url, savePath: mpPath });
        }
        const res: any = await invoke("add_model", { path: savePath, flags: { uncen: !!model.uncen, vision: !!model.vision, audio: !!model.audio } });
        if (res?.warning) showToast(res.warning, "error");
        await this.loadConfig(); showToast(`Модель ${model.name} скачана!`, "success");
      } catch(e) { showToast(`Ошибка: ${e}`, "error"); void trackError("settings.downloadModel", e); }
      finally { this.el.btnDownloadModel.disabled = false; this.el.downloadProgressContainer.style.display = "none"; }
    });

    this.el.btnAutoDownload?.addEventListener("click", async () => {
      try {
        const info: any = await invoke("get_auto_download_info");
        this.el.modalModelName.innerText = info.size_gb ? `${info.model_name} (${info.size_gb} GB)` : info.model_name;
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
        this.resetDownloadUi();
        await invoke("auto_download_default_model", { savePath: info.save_path });
        await this.loadConfig();
        showToast(`Модель ${info.model_name} скачана!`, "success");
      } catch(e) { showToast(`Ошибка: ${e}`, "error"); void trackError("settings.autoDownload", e); }
      finally { this.el.btnDownloadModel.disabled = false; this.el.downloadProgressContainer.style.display = "none"; }
    });

    // ─── Движок запуска нейромоделей (llamacpp) ───
    this.el.engineVariantSelect?.addEventListener("change", () => {
      // Подсказка под дропдауном + показать/спрятать кнопку «Применить»
      const st: any = { available_variants: [], installed_variants: [], resolved_variant: "" };
      const prevValue = this.el.engineVariantSelect.dataset.applied || "auto";
      this.updateEngineVariantHint(st, this.el.engineVariantSelect.value);
      this.el.btnApplyEngineVariant.style.display =
        this.el.engineVariantSelect.value === prevValue ? "none" : "inline-block";
      void invoke("get_engine_status").then((s: any) => {
        this.updateEngineVariantHint(s, this.el.engineVariantSelect.value);
      }).catch(() => {});
    });

    this.el.btnApplyEngineVariant?.addEventListener("click", async () => {
      const btn = this.el.btnApplyEngineVariant;
      const variant = this.el.engineVariantSelect.value;
      btn.disabled = true;
      let installedVariants: string[] = [];
      try {
        const st: any = await invoke("get_engine_status");
        installedVariants = st.installed_variants || [];
      } catch { /* не критично — прогресс покажем в любом случае */ }
      if (!installedVariants.includes(variant)) {
        this.el.engineProgressContainer.style.display = "block";
        this.el.engineStatusLabel.innerText = "Скачивание бекенда...";
        this.el.engineProgressBar.style.width = "0%";
      }
      try {
        await invoke("set_engine_variant", { variant });
        showToast(variant === "auto" ? "Бекенд: авто-подбор." : "Бекенд применён.", "success");
      } catch (e) {
        showToast(`Ошибка смены бекенда: ${e}`, "error");
        void trackError("settings.engineVariant", e);
      } finally {
        btn.disabled = false;
        this.el.engineProgressContainer.style.display = "none";
        await this.refreshEngineStatus();
      }
    });

    this.el.btnInstallEngine?.addEventListener("click", async () => {
      const btn = this.el.btnInstallEngine;
      btn.disabled = true;
      this.el.engineProgressContainer.style.display = "block";
      this.el.engineStatusLabel.innerText = "Подготовка...";
      this.el.engineProgressBar.style.width = "0%";
      try {
        await invoke("install_llamacpp");
        await this.refreshEngineStatus();
        showToast("Движок llamacpp установлен!", "success");
      } catch (e) {
        showToast(`Ошибка установки движка: ${e}`, "error");
        void trackError("settings.installEngine", e);
      } finally {
        btn.disabled = false;
        this.el.engineProgressContainer.style.display = "none";
      }
    });

    this.el.btnCheckEngineUpdate?.addEventListener("click", async () => {
      const btn = this.el.btnCheckEngineUpdate;
      const status = this.el.engineStatus;
      btn.disabled = true;
      this.el.btnInstallEngineUpdate.style.display = "none";
      try {
        const newTag: string | null = await invoke("check_engine_update");
        if (newTag) {
          status.textContent = `Доступно обновление движка: ${newTag}`;
          this.el.btnInstallEngineUpdate.style.display = "inline-block";
        } else {
          status.textContent = "Движок llamacpp актуален";
        }
      } catch (e) {
        status.textContent = "";
        showToast(`Ошибка проверки обновления движка: ${e}`, "error");
        void trackError("settings.checkEngineUpdate", e);
      } finally {
        btn.disabled = false;
      }
    });

    this.el.btnInstallEngineUpdate?.addEventListener("click", async () => {
      const btn = this.el.btnInstallEngineUpdate;
      btn.disabled = true;
      this.el.engineProgressContainer.style.display = "block";
      this.el.engineStatusLabel.innerText = "Обновление...";
      this.el.engineProgressBar.style.width = "0%";
      try {
        await invoke("install_engine_update");
        this.el.btnInstallEngineUpdate.style.display = "none";
        await this.refreshEngineStatus();
        showToast("Движок llamacpp обновлён.", "success");
      } catch (e) {
        showToast(`Ошибка обновления движка: ${e}`, "error");
        void trackError("settings.installEngineUpdate", e);
      } finally {
        btn.disabled = false;
        this.el.engineProgressContainer.style.display = "none";
      }
    });

    this.el.btnRemoveEngine?.addEventListener("click", async () => {
      if (!confirm("Удалить движок llamacpp? Будет освобождено ~1 ГБ на диске. GPU-ускорение отключится.")) return;
      try {
        await invoke("remove_engine");
        await this.refreshEngineStatus();
        showToast("Движок llamacpp удалён.", "success");
      } catch (e) { showToast(`Ошибка удаления движка: ${e}`, "error"); void trackError("settings.removeEngine", e); }
    });

    this.el.btnSetEngineDir?.addEventListener("click", async () => {
      try {
        const sel = await openDialog({ directory: true });
        if (!sel) return;
        await invoke("set_engine_dir", { path: sel as string });
        await this.refreshEngineStatus();
        showToast("Путь движка изменён.", "success");
      } catch (e) { showToast(`Ошибка изменения пути: ${e}`, "error"); void trackError("settings.setEngineDir", e); }
    });
  }

  private bindTauriEvents() {
    listen("download_progress", (e: any) => {
      const { downloaded, total, speed_bps } = e.payload;
      this.lastDownloadActivity = Date.now();
      this.el.downloadProgressContainer.classList.remove("download-stalled");
      const pct = total > 0 ? (downloaded / total) * 100 : 0;
      this.el.downloadProgressBar.style.width = `${pct}%`;
      const speed = formatSpeed(speed_bps);
      this.el.downloadStatusLabel.innerText = `${formatBytes(downloaded)} / ${total > 0 ? formatBytes(total) : "?"}${total > 0 ? ` (${pct.toFixed(0)}%)` : ""} · ${speed}`;
    });
    listen("engine_progress", (e: any) => { const { downloaded, total } = e.payload; const pct = total > 0 ? (downloaded / total) * 100 : 0; this.el.engineProgressBar.style.width = `${pct}%`; this.el.engineStatusLabel.innerText = `${(downloaded/1024/1024).toFixed(1)} MB / ${(total/1024/1024).toFixed(1)} MB`; });
  }

  /// Сброс прогресс-бара перед началом новой загрузки.
  private resetDownloadUi() {
    this.lastDownloadActivity = 0;
    this.el.downloadProgressContainer.style.display = "block";
    this.el.downloadProgressContainer.classList.remove("download-stalled");
    this.el.downloadProgressBar.style.width = "0%";
    this.el.downloadStatusLabel.innerText = "Подключение...";
  }

  private bindBusEvents() {
    bus.on("model:changed", (modelPath: string) => {
      if (this.el.modelSelect.value === modelPath) this.loadModelParams();
    });
  }
}