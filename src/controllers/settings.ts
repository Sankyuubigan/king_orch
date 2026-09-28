import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getAllCapabilities, getModelParams, resetModelParams, setModelParams } from "@my-tauri-plugins/plugin-llama-engine";
import { getCombos as getCloudRouterCombos, type ComboInfo } from "@my-tauri-plugins/plugin-cloud-routers";
import { store } from "../store";
import { bus } from "../events";
import { showToast } from "../ui";
import { setTelemetryEnabled, trackError } from "../telemetry";
import { CLOUD_ROUTER_PREFIXES } from "../utils";
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
  dataDirDisplay: HTMLElement;
  btnChangeDataDir: HTMLButtonElement;
}

export class SettingsController {
  private el: SettingsElements;
  /// Кэш комбо облачных роутеров — пишем в store.cloudRouterCombos.
  private cloudRouterCombos: Record<string, ComboInfo[]> = {};
  private cloudRouterCombosRequested = false;

  constructor(el: SettingsElements) {
    this.el = el;
    this.bindDomEvents();
    this.bindBusEvents();
    window.addEventListener("cloud-routers:combos-changed", (e) => {
      const detail = (e as CustomEvent<{ router?: string; combos?: ComboInfo[] }>).detail;
      if (detail && detail.router && Array.isArray(detail.combos)) {
        this.cloudRouterCombos[detail.router] = detail.combos;
        store.cloudRouterCombos[detail.router] = detail.combos.map(c => ({ name: c.name, models: c.models || [] }));
        bus.emit("model-catalog-changed");
        return;
      }
      void this.refreshCloudRouterCombos();
    });
  }

  /**
   * Загрузка параметров сэмплинга текущей модели в общие слайдеры.
   * Модель берётся из АКТИВНОЙ чат-вкладки (селекты моделей — per-tab).
   */
  async loadModelParams() {
    const p = getActiveChatController()?.modelSelectValue ?? store.lastModel;
    if (!p) return;
    // Комбо облачных роутеров — облачная модель: локальные параметры сэмплинга не храним.
    if (Object.values(CLOUD_ROUTER_PREFIXES).some(prefix => p.startsWith(prefix))) return;
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
    if (Object.values(CLOUD_ROUTER_PREFIXES).some(prefix => p.startsWith(prefix))) return;
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

  private async resetDefaults() {
    try {
      const model = getActiveChatController()?.modelSelectValue ?? store.lastModel;
      const modelReset = model && !Object.values(CLOUD_ROUTER_PREFIXES).some(prefix => model.startsWith(prefix))
        ? (async () => {
            await resetModelParams(model);
            await this.loadModelParams();
          })()
        : Promise.resolve();
      const [_, maxGen] = await Promise.all([
        modelReset,
        invoke<number>("reset_max_gen_tokens"),
      ]);
      this.el.maxGenSlider.value = String(maxGen);
      this.el.maxGenValue.innerText = String(maxGen);
      showToast("Параметры сброшены.", "success");
    } catch (e) {
      showToast(`Не удалось сбросить параметры: ${e}`, "error");
      void trackError("settings.resetDefaults", e);
    }
  }

  /// Загружает актуальные возможности всех моделей (живой
  /// `get_all_capabilities` плагина, единый источник правды) и пишет в Store —
  /// дропдауны всех чат-вкладок перерисовываются из store.capabilities.
  private async refreshCapabilities() {
    try {
      store.capabilities = await getAllCapabilities();
    } catch (_) { store.capabilities = {}; }
    bus.emit("model-catalog-changed");
  }

  /// Ленивая подгрузка комбо облачных роутеров: запрашиваем с сервера (поднимает его,
  /// только если включён автозапуск). Результат кэшируем.
  private async ensureCloudRouterCombos() {
    if (this.cloudRouterCombosRequested) return;
    await this.refreshCloudRouterCombos();
  }

  /// Перечитывает комбо облачных роутеров и пишет в store.cloudRouterCombos.
  private async refreshCloudRouterCombos() {
    this.cloudRouterCombosRequested = true;
    try {
      const routers = ["9router", "extremerouter", "omniroute"];
      for (const router of routers) {
        try {
          const combos = await getCloudRouterCombos(router as any);
          this.cloudRouterCombos[router] = combos;
          store.cloudRouterCombos[router] = combos.map(c => ({ name: c.name, models: c.models || [] }));
        } catch (_) {
          this.cloudRouterCombos[router] = [];
          store.cloudRouterCombos[router] = [];
        }
      }
    } catch (_) {
      this.cloudRouterCombosRequested = false;
      return;
    }
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

  private applyTheme(theme: string) {
    const normalized = theme === "light" ? "light" : "dark";
    this.el.themeSelect.value = normalized;
    document.documentElement.setAttribute("data-theme", normalized);
    const key = document.documentElement.dataset.themeCacheKey;
    if (!key) {
      void trackError("settings.theme-cache", new Error("Отсутствует ключ кэша темы"));
      return;
    }
    try {
      localStorage.setItem(key, normalized);
    } catch (e) {
      void trackError("settings.theme-cache", e);
    }
  }

  async loadConfigCore() {
    store.agentsReady = false;
    try {
      const config: any = await invoke("get_config");
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
      }
      if (config.theme) this.applyTheme(config.theme);
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
      void this.loadDataDir();
      return config;
    } catch (e) {
      showToast(`Ошибка: ${e}`, "error");
      void trackError("settings.loadConfigCore", e);
      return null;
    }
  }

  async hydrateCatalogs(config: any) {
    await Promise.all([
      this.refreshCapabilities(),
      this.loadAgents(config.last_agent),
      this.loadModelParams(),
    ]);
    bus.emit("config:loaded", config);
    void this.ensureNineRouterCombos();
  }

  async loadConfig() {
    const config = await this.loadConfigCore();
    if (!config) return null;
    await this.hydrateCatalogs(config);
    return config;
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
      store.agentsReady = true;
      bus.emit("model-catalog-changed");
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
    this.el.themeSelect?.addEventListener("change", async () => { this.applyTheme(this.el.themeSelect.value); await invoke("set_theme", { theme: this.el.themeSelect.value }); });
    const sliders: [HTMLInputElement, HTMLElement][] = [[this.el.tempSlider, this.el.tempValue],[this.el.topkSlider, this.el.topkValue],[this.el.toppSlider, this.el.toppValue],[this.el.minpSlider, this.el.minpValue],[this.el.reppenSlider, this.el.reppenValue],[this.el.prespenSlider, this.el.prespenValue]];
    for (const [s, l] of sliders) s?.addEventListener("input", () => { l.innerText = s.value; this.saveModelParams(); });
    this.el.btnResetParams?.addEventListener("click", () => { void this.resetDefaults(); });
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
    this.el.btnChangeDataDir?.addEventListener("click", () => { void this.onChangeDataDir(); });
  }

  /** Загружает текущий путь общего хранилища (`KingOrchData`) в плашку. */
  private async loadDataDir() {
    try {
      const dir = await invoke<string>("get_data_dir");
      this.el.dataDirDisplay.textContent = dir || "не выбрано";
    } catch (e) {
      this.el.dataDirDisplay.textContent = "ошибка загрузки";
      void trackError("settings.loadDataDir", e);
    }
  }

  /** Диалог выбора папки → set_data_dir → обновление плашки. */
  private async onChangeDataDir() {
    try {
      const sel = await openDialog({ directory: true });
      if (!sel) return;
      const path = Array.isArray(sel) ? sel[0] : sel;
      if (!path) return;
      const saved = await invoke<string>("set_data_dir", { path });
      this.el.dataDirDisplay.textContent = saved;
      showToast("Путь хранилища изменён. Движки обновят его после переоткрытия вкладки «Движки».", "success");
      bus.emit("model-catalog-changed");
      window.dispatchEvent(new CustomEvent("cloud-routers:combos-changed"));
    } catch (e) {
      showToast(`Ошибка смены пути: ${e}`, "error");
      void trackError("settings.onChangeDataDir", e);
    }
  }

  private bindBusEvents() {
    // Параметры сэмплинга = АКТИВНАЯ вкладка (селекты моделей per-tab):
    // меняется выбор в активной вкладке → перечитываем параметры.
    bus.on("model:changed", (modelPath: string) => {
      if (getActiveChatController()?.modelSelectValue === modelPath) this.loadModelParams();
    });
  }
}