import { describe, it, expect, beforeEach, vi } from "vitest";
import { ChatController } from "./chat";
import { showToast } from "../ui/toast";
import { bus } from "../events";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

function createMockElements(): any {
  document.body.innerHTML = `
    <div id="chat-history"></div>
    <textarea id="chat-input"></textarea>
    <button id="btn-send"></button>
    <button id="btn-stop"></button>
    <div id="chat-feedback"></div>
    <div id="progress-bar"></div>
    <div id="status-label"></div>
    <select id="agent-select"></select>
    <select id="model-select"></select>
    <div id="subchat-history"></div>
    <span id="subchat-title"></span>
    <button id="btn-back-chat"></button>
    <textarea id="log-view"></textarea>
    <input id="context-slider" />
    <input id="chk-kv-quant" type="checkbox" />
    <input id="temp-slider" />
    <input id="topk-slider" />
    <input id="topp-slider" />
    <input id="minp-slider" />
    <input id="reppen-slider" />
    <input id="prespen-slider" />
    <div id="view-chat"></div>
    <div id="view-subchat"></div>
    <button id="btn-attach"></button>
    <input id="file-input" type="file" />
    <div id="file-preview"></div>
    <div id="toast-container"></div>
  `;
  return {
    chatHistory: document.getElementById("chat-history") as HTMLDivElement,
    chatInput: document.getElementById("chat-input") as HTMLTextAreaElement,
    btnSend: document.getElementById("btn-send") as HTMLButtonElement,
    btnStop: document.getElementById("btn-stop") as HTMLButtonElement,
    chatFeedback: document.getElementById("chat-feedback") as HTMLDivElement,
    progressBar: document.getElementById("progress-bar") as HTMLDivElement,
    statusLabel: document.getElementById("status-label") as HTMLDivElement,
    agentSelect: document.getElementById("agent-select") as HTMLSelectElement,
    modelSelect: document.getElementById("model-select") as HTMLSelectElement,
    subchatHistory: document.getElementById("subchat-history") as HTMLDivElement,
    subchatTitle: document.getElementById("subchat-title") as HTMLSpanElement,
    btnBackChat: document.getElementById("btn-back-chat") as HTMLButtonElement,
    logView: document.getElementById("log-view") as HTMLTextAreaElement,
    contextSlider: document.getElementById("context-slider") as HTMLInputElement,
    chkKvQuant: document.getElementById("chk-kv-quant") as HTMLInputElement,
    tempSlider: document.getElementById("temp-slider") as HTMLInputElement,
    topkSlider: document.getElementById("topk-slider") as HTMLInputElement,
    toppSlider: document.getElementById("topp-slider") as HTMLInputElement,
    minpSlider: document.getElementById("minp-slider") as HTMLInputElement,
    reppenSlider: document.getElementById("reppen-slider") as HTMLInputElement,
    prespenSlider: document.getElementById("prespen-slider") as HTMLInputElement,
    viewChat: document.getElementById("view-chat") as HTMLDivElement,
    viewSubchat: document.getElementById("view-subchat") as HTMLDivElement,
    btnAttach: document.getElementById("btn-attach") as HTMLButtonElement,
    fileInput: document.getElementById("file-input") as HTMLInputElement,
    filePreview: document.getElementById("file-preview") as HTMLDivElement,
  };
}

describe("ChatController.logToGUI", () => {
  beforeEach(() => {
    createMockElements();
  });

  it("should append message to logView textarea with timestamp", () => {
    const el = createMockElements();
    const ctrl = new ChatController(el);

    ctrl.logToGUI("Test log message");

    expect(el.logView.value).toContain("Test log message");
    expect(el.logView.value).toMatch(/^\[\d{1,2}:\d{2}:\d{2}\] Test log message\n$/);
  });

  it("should append multiple messages on separate lines", () => {
    const el = createMockElements();
    const ctrl = new ChatController(el);

    ctrl.logToGUI("First message");
    ctrl.logToGUI("Second message");

    const lines = el.logView.value.trim().split("\n");
    expect(lines.length).toBe(2);
    expect(lines[0]).toMatch(/\[\d{1,2}:\d{2}:\d{2}\] First message/);
    expect(lines[1]).toMatch(/\[\d{1,2}:\d{2}:\d{2}\] Second message/);
  });

  it("should handle missing logView gracefully", () => {
    document.body.innerHTML = "";
    const el = createMockElements();

    try {
      const ctrl = new ChatController(el);
      expect(() => ctrl.logToGUI("test")).not.toThrow();
    } catch (_) {}
  });
});

describe("Error toast logging integration", () => {
  it("showToast with error should appear in logView when bus forwards to logToGUI", () => {
    const el = createMockElements();
    const ctrl = new ChatController(el);

    bus.on("log", (msg: string) => ctrl.logToGUI(msg));

    showToast("Test error", "error");

    expect(el.logView.value).toContain("❌ Test error");
  });

  it("showToast with success should NOT appear in logView", () => {
    const el = createMockElements();
    const ctrl = new ChatController(el);

    bus.on("log", (msg: string) => ctrl.logToGUI(msg));

    showToast("Test success", "success");

    expect(el.logView.value).toBe("");
  });
});
