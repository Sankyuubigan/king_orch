import { describe, it, expect, beforeEach, vi } from "vitest";
import { showToast } from "./toast";
import { bus } from "../events";

function setupDOM() {
  document.body.innerHTML = '<div id="toast-container"></div>';
}

describe("showToast", () => {
  beforeEach(() => {
    setupDOM();
  });

  it("should emit 'log' bus event with error prefix when type is 'error'", () => {
    const logSpy = vi.fn();
    bus.on("log", logSpy);

    showToast("Test error message", "error");

    expect(logSpy).toHaveBeenCalledWith("❌ Test error message");
  });

  it("should NOT emit 'log' bus event when type is 'success'", () => {
    const logSpy = vi.fn();
    bus.on("log", logSpy);

    showToast("Test success", "success");

    expect(logSpy).not.toHaveBeenCalled();
  });

  it("should NOT emit 'log' bus event when type is 'info'", () => {
    const logSpy = vi.fn();
    bus.on("log", logSpy);

    showToast("Test info", "info");

    expect(logSpy).not.toHaveBeenCalled();
  });

  it("should create a toast element in the container", () => {
    showToast("Test toast", "info");

    const container = document.getElementById("toast-container");
    expect(container?.children.length).toBe(1);
    expect(container?.children[0].textContent).toBe("Test toast");
    expect(container?.children[0].className).toContain("toast-info");
  });

  it("should create error toast with toast-error class", () => {
    showToast("Error msg", "error");

    const container = document.getElementById("toast-container");
    expect(container?.children[0].className).toContain("toast-error");
  });

  it("should do nothing if toast-container does not exist", () => {
    document.body.innerHTML = "";

    const logSpy = vi.fn();
    bus.on("log", logSpy);

    expect(() => showToast("No container", "error")).not.toThrow();
    expect(logSpy).not.toHaveBeenCalled();
  });

  it("should default to error type when no type provided", () => {
    const logSpy = vi.fn();
    bus.on("log", logSpy);

    showToast("Default error");

    const container = document.getElementById("toast-container");
    expect(container?.children[0].className).toContain("toast-error");
    expect(logSpy).toHaveBeenCalledWith("❌ Default error");
  });
});
