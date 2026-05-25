import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

/**
 * Сервис работы с параметрами моделей и каталогом.
 * Выделен из main.ts по принципу SRP.
 */

export async function getModelParams(modelPath: string): Promise<any> {
    if (!modelPath) return {};
    return await invoke("get_model_params", { modelPath });
}

export async function setModelParams(modelPath: string, params: any): Promise<void> {
    if (!modelPath) return;
    await invoke("set_model_params", { modelPath, params });
}

export async function resetModelParams(modelPath: string): Promise<any> {
    if (!modelPath) return {};
    return await invoke("reset_model_params", { modelPath });
}

export async function loadModelsCatalog(): Promise<any[]> {
    try {
        return await invoke("get_models_catalog");
    } catch (e) {
        return [];
    }
}

export async function downloadModelAction(model: any): Promise<string | null> {
    const savePath = await save({ defaultPath: `${model.name}.gguf`, filters: [{ name: "GGUF", extensions: ["gguf"] }] });
    if (!savePath) return null;
    
    await invoke("download_model", { url: model.download_url, savePath });
    await invoke("add_model", { path: savePath });
    
    return savePath;
}