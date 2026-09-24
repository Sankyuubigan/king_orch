import { invoke } from "@tauri-apps/api/core";
import { getAllCapabilities, getModelParams, resetModelParams, setModelParams } from "@my-tauri-plugins/plugin-llama-engine";
import { getCombos as getNineRouterCombos, type ComboInfo } from "@my-tauri-plugins/plugin-9router";
import { store } from "../store";
import { bus } from "../events";
import { showToast } from "../ui";
import { setTelemetryEnabled, trackError } from "../telemetry";
import { NINE_ROUTER_MODEL_PREFIX } from "../utils";
import { getActiveChatController } from "./tabs";

export interface SettingsElements {
  maxGenSlider: HTMLInputElement; maxGenValue: HTMLElement;
  themeSelect: HTMLSelectElement;
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
  /// Кэш комбо 9Router (облачные модели) — пишем в store.nineRouterCombos.
  private nineRouterCombos: ComboInfo[] = [];
  private nineRouterCombosRequested = false;

  constructor(el: SettingsElements) {
    this.el = el;
    this.bindDomEvents();
    this.bindBusEvents();
    // Панель 9Router (настройки) сообщает об изменении комбо (кнопки
    // «Обновить комбо»/установка/смена пути) — перечитываем дропдаун хоста.
    window.addEventListener("9router:combos-changed", () => { void this.refreshNineRouterCombos(); });
  }

  /**
   * Загрузка параметров сэмплинга текущей модели в общие слайдеры.
   * Модель берётся из АКТИВНОЙ чат-вкладки (селекты моделей — per-tab).
   */
  async loadModelParams() {
    const p = getActiveChatController()?.modelSelectValue ?? store.lastModel;
    if (!p) return;
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
    const p = getActiveChatController()?.modelSelectValue ?? store.lastModel;
    if (!p) return;
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
  /// `get_all_capabilities` плагина, единый источник правды) и пишет в Store —
  /// дропдауны всех чат-вкладок перерисовываются из store.capabilities.
  private async refreshCapabilities() {
    try {
      store.capabilities = await getAllCapabilities();
    } catch (_) { store.capabilities = {}; }
  }

  /// Ленивая подгрузка комбо 9Router: `get_combos` сам поднимает сервер по
  /// требованию (шлюз ленивый), результат кэшируем — повторно не дёргаем.
  private async ensureNineRouterCombos() {
    if (this.nineRouterCombosRequested) return;
    await this.refreshNineRouterCombos();
  }

  /// Перечитывает комбо 9Router и пишет в store.nineRouterCombos.
  /// При ошибке/пустом списке НЕ защёлкивает флаг — даёт повторную попытку.
  private async refreshNineRouterCombos() {
    this.nineRouterCombosRequested = true;
    try {
      this.nineRouterCombos = await getNineRouterCombos();
    } catch (_) {
      this.nineRouterCombos = [];
      this.nineRouterCombosRequested = false;
      store.nineRouterCombos = [];
      return;
    }
    if (!this.nineRouterCombos.length) {
      this.nineRouterCombosRequested = false;
      store.nineRouterCombos = [];
      return;
    }
    store.nineRouterCombos = this.nineRouterCombos.map(c => ({ name: c.name, models: c.models || [] }));
    bus.emit("model-catalog-changed");
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

  async loadConfig() {
    bus.emit("log", "Загрузка конфигурации...");
    try {
      const config: any = await invoke("get_config");
      // Общие каталоги моделей/агентов — SSOT в Store (селекты живут по-вкладке).
      store.models = Array.isArray(config.models) ? config.models : store.models;
      store.lastModel = config.last_model ?? null;
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
      if (config.workdir) store.workdir = config.workdir;
      if (config.max_gen_tokens) { this.el.maxGenSlider.value = config.max_gen_tokens.toString(); this.el.maxGenValue.innerText = config.max_gen_tokens.toString(); }
      if (config.chat_font_scale !== undefined) {
        const pct = Math.round(config.chat_font_scale * 100);
        this.el.chatFontSlider.value = pct.toString();
        this.el.chatFontValue.innerText = `${pct}%`;
        this.applyChatFontScale(config.chat_font_scale);
        store.workdir = store.workdir;
      }
      if (config.theme) { this.el.themeSelect.value = config.theme; document.documentElement.setAttribute('data-theme', config.theme); }
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
      await Promise.all([
        this.refreshCapabilities(),
        this.loadAgents(config.last_agent),
        this.loadModelParams(),
      ]);
      bus.emit("config:loaded", config);
      void this.ensureNineRouterCombos();
      return config;
    } catch(e) { showToast(`Ошибка: ${e}`, "error"); void trackError("settings.loadConfig", e); return null; }
  }

  /** Применяет масштаб шрифта чата (1.0 = 100%) через CSS-переменную. */
  private applyChatFontScale(scale: number) {
    document.documentElement.style.setProperty("--chat-font-scale", String(scale));
  }

  private async loadAgents(lastAgent?: string) {
    try {
      const entries: any[] = await invoke("get_agents");
      store.agents = entries;
      store.lastAgent = lastAgent ?? null;
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
    this.el.themeSelect?.addEventListener("change", async () => { document.documentElement.setAttribute('data-theme', this.el.themeSelect.value); await invoke("set_theme", { theme: this.el.themeSelect.value }); });
    const sliders: [HTMLInputElement, HTMLElement][] = [[this.el.tempSlider, this.el.tempValue],[this.el.topkSlider, this.el.topkValue],[this.el.toppSlider, this.el.toppValue],[this.el.minpSlider, this.el.minpValue],[this.el.reppenSlider, this.el.reppenValue],[this.el.prespenSlider, this.el.prespenValue]];
    for (const [s, l] of sliders) s?.addEventListener("input", () => { l.innerText = s.value; this.saveModelParams(); });
    this.el.btnResetParams?.addEventListener("click", async () => { const p = getActiveChatController()?.modelSelectValue ?? store.lastModel; if (!p) return; await resetModelParams(p); await this.loadModelParams(); showToast("Параметры сброшены.", "success"); });
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
      bus.emit("model-catalog-changed");
    });
    this.el.chkErrorReports?.addEventListener("change", async () => {
      const val = this.el.chkErrorReports.checked;
      // Мгновенно синхронизируем состояние телеметрии и сохраняем настройку.
      setTelemetryEnabled(val);
      await invoke("set_config_value", { key: "allow_error_reports", value: val });
    });
  }

  private bindBusEvents() {
    // Параметры сэмплинга = АКТИВНАЯ вкладка (селекты моделей per-tab):
    // меняется выбор в активной вкладке → перечитываем параметры.
    bus.on("model:changed", (modelPath: string) => {
      if (getActiveChatController()?.modelSelectValue === modelPath) this.loadModelParams();
    });
  }
}