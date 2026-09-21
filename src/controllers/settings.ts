import { invoke } from "@tauri-apps/api/core";
import { getAllCapabilities, getModelParams, resetModelParams, setModelParams } from "@my-tauri-plugins/plugin-llama-engine";
import { getCombos as getNineRouterCombos, type ComboInfo } from "@my-tauri-plugins/plugin-9router";
import { store } from "../store";
import { bus } from "../events";
import { showToast } from "../ui";
import { setTelemetryEnabled, trackError } from "../telemetry";
import { NINE_ROUTER_MODEL_PREFIX } from "../utils";

export interface SettingsElements {
  modelSelect: HTMLSelectElement;
  agentSelect: HTMLSelectElement;
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
  chkShowAdvanced: HTMLInputElement;
  chkShowFolderAgents: HTMLInputElement;
  chkErrorReports: HTMLInputElement;
  translatorModelSelect: HTMLSelectElement;
  translatorLangSelect: HTMLSelectElement;
  chatFontSlider: HTMLInputElement;
  chatFontValue: HTMLElement;
}

export class SettingsController {
  private el: SettingsElements;
  private capMap: Record<string, { uncen: boolean; vision: boolean; audio: boolean }> = {};
  /// Кэш комбо 9Router (облачные модели) для дропдауна — см. updateModelSelect.
  private nineRouterCombos: ComboInfo[] = [];
  private nineRouterCombosRequested = false;

  constructor(el: SettingsElements) {
    this.el = el;
    this.bindDomEvents();
    this.bindBusEvents();
  }

  async loadModelParams() {
    const p = this.el.modelSelect.value; if (!p) return;
    // Комбо 9Router — облачная модель: локальные параметры сэмплинга не храним.
    if (p.startsWith(NINE_ROUTER_MODEL_PREFIX)) return;
    let params: any;
    try {
      params = await getModelParams(p);
    } catch (e) {
      showToast(`Не удалось прочитать параметры модели: ${e}`, "error");
      return;
    }
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
    if (p.startsWith(NINE_ROUTER_MODEL_PREFIX)) return;
    const base = store.currentModelParams;
    await setModelParams(p, {
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
    });
  }

  /// Загружает актуальные возможности всех моделей (живой
  /// `get_all_capabilities` плагина, единый источник правды) и перерисовывает
  /// список моделей. Заменяет использование устаревающего `model_meta`.
  private async refreshModelSelect(config: any) {
    try {
      this.capMap = await getAllCapabilities();
    } catch (_) { this.capMap = {}; }
    this.updateModelSelect(config);
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
    this.renderNineRouterOptions();
    void this.ensureNineRouterCombos();
  }

  /// Рисует optgroup «9Router (облако)» поверх локальных моделей из кэша комбо.
  private renderNineRouterOptions() {
    if (!this.nineRouterCombos.length) return;
    const group = document.createElement("optgroup");
    group.label = "9Router (облако)";
    for (const c of this.nineRouterCombos) {
      const o = document.createElement("option");
      o.value = `${NINE_ROUTER_MODEL_PREFIX}${c.name}`;
      o.text = `☁️ ${c.name}`;
      group.appendChild(o);
    }
    this.el.modelSelect.appendChild(group);
  }

  /// Ленивая подгрузка комбо 9Router. `get_combos` сам поднимает сервер по
  /// требованию (шлюз ленивый), но результат кэшируем — повторно не дёргаем.
  private async ensureNineRouterCombos() {
    if (this.nineRouterCombosRequested) return;
    this.nineRouterCombosRequested = true;
    try {
      this.nineRouterCombos = await getNineRouterCombos();
    } catch (_) {
      // 9Router не установлен/недоступен — облачные комбо просто не показываем.
      this.nineRouterCombos = [];
    }
    if (!this.nineRouterCombos.length) return;
    const selected = this.el.modelSelect.value;
    this.el.modelSelect.querySelector('optgroup[label="9Router (облако)"]')?.remove();
    this.renderNineRouterOptions();
    if (selected) this.el.modelSelect.value = selected;
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

  async loadConfig() {
    bus.emit("log", "Загрузка конфигурации...");
    try {
      const config: any = await invoke("get_config");
      await this.refreshModelSelect(config);
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
      if (config.context_size) store.contextSize = config.context_size;
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
      await this.loadModelParams();
    } catch(e) { showToast(`Ошибка: ${e}`, "error"); void trackError("settings.loadConfig", e); }
  }

  /** Применяет масштаб шрифта чата (1.0 = 100%) через CSS-переменную. */
  private applyChatFontScale(scale: number) {
    document.documentElement.style.setProperty("--chat-font-scale", String(scale));
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

  private bindDomEvents() {
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
    this.el.btnResetParams?.addEventListener("click", async () => { const p = this.el.modelSelect.value; if (!p) return; await resetModelParams(p); await this.loadModelParams(); showToast("Параметры сброшены.", "success"); });
    this.el.modelSelect?.addEventListener("change", async () => {
      const v = this.el.modelSelect.value;
      // Комбо 9Router не пишем как last_model локального движка.
      if (v.startsWith(NINE_ROUTER_MODEL_PREFIX)) return;
      await invoke("set_last_model", { path: v });
      await this.loadModelParams();
    });
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
  }

  private bindBusEvents() {
    bus.on("model:changed", (modelPath: string) => {
      if (this.el.modelSelect.value === modelPath) this.loadModelParams();
    });
  }
}