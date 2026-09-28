import { onUpdateState } from "@my-tauri-plugins/plugin-llama-engine";
import { onUpdateState as onImageUpdateState } from "@my-tauri-plugins/plugin-image-engine";
import { onUpdateState as onRoutersUpdateState } from "@my-tauri-plugins/plugin-cloud-routers";
import { logFront } from "@my-tauri-plugins/plugin-logs";
import { bus } from "../events";

export interface PluginUpdateState {
  hasUpdate: boolean;
  tag?: string;
}

interface UpdateStore {
  app: { hasUpdate: boolean };
  plugins: Map<string, PluginUpdateState>;
}

let store: UpdateStore = {
  app: { hasUpdate: false },
  plugins: new Map(),
};

let navMenu: { setBadge: (id: string, visible: boolean) => void } | null = null;

export function initUpdateBadgeTargets(t: { menuEngines: { setBadge: (id: string, visible: boolean) => void } | null }) {
  navMenu = t.menuEngines;
}

export function initUpdateWatchers(): () => void {
  const unsubs: (() => void)[] = [];

  unsubs.push(onUpdateState((s) => {
    store.plugins.set("llama-engine", s);
    renderUpdateBadges();
  }));
  unsubs.push(onImageUpdateState((s) => {
    store.plugins.set("image-engine", s);
    renderUpdateBadges();
  }));
  unsubs.push(onRoutersUpdateState((s) => {
    store.plugins.set("cloud-routers", s);
    renderUpdateBadges();
  }));

  unsubs.push(bus.on("app:update-available", (hasUpdate: boolean) => {
    store.app.hasUpdate = hasUpdate;
    logFront(`[updates] приложение ${hasUpdate ? "имеет обновление" : "актуально"}`);
    renderUpdateBadges();
  }));

  unsubs.push(bus.on("updates:render", () => renderUpdateBadges()));

  logFront("[updates] watchers инициализированы");
  return () => unsubs.forEach((u) => u());
}

export function renderUpdateBadges() {
  const hasEngineUpdate = ["llama-engine", "image-engine"].some(
    (id) => store.plugins.get(id)?.hasUpdate
  );

  if (navMenu) {
    navMenu.setBadge("engines", hasEngineUpdate);
    navMenu.setBadge("settings", store.app.hasUpdate);
  }

  const enginesBtn = document.querySelector<HTMLElement>('.settings-nav-btn[data-section="engines"]');
  if (enginesBtn) toggleDot(enginesBtn, hasEngineUpdate);

  const settingsBtn = document.querySelector<HTMLElement>('.settings-nav-btn[data-section="settings"]');
  if (settingsBtn) toggleDot(settingsBtn, store.app.hasUpdate);

  const aboutTitle = document.querySelector("about-updates-panel")
    ?.closest(".group-box")
    ?.querySelector<HTMLElement>(".group-title");
  if (aboutTitle) toggleDot(aboutTitle, store.app.hasUpdate);
}

function toggleDot(container: HTMLElement, visible: boolean) {
  let dot = container.querySelector<HTMLElement>(".update-dot");
  if (visible && !dot) {
    dot = document.createElement("span");
    dot.className = "update-dot";
    container.appendChild(dot);
  } else if (!visible && dot) {
    dot.remove();
  }
}
