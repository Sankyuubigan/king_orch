#!/usr/bin/env -S deno run --allow-all
// downloader.ts — MCP-сервер для скачивания файлов с фоллбэками.
//
// Режимы:
//   1. MCP-сервер (stdin/stdout JSON-RPC) — вызывается из MCP Pool
//   2. CLI: deno run --allow-all downloader.ts --download-cli <url> <path>
//      → Скачивает файл и выводит "OK:<bytes>" или "ERROR:<msg>"

import { createMcpServer, log } from "./mcp_base.ts";
import https from "node:https";
import http from "node:http";
import { Buffer } from "node:buffer";
import { setTimeout as sleep } from "node:timers/promises";

// ── CLI mode ──
const args = Deno.args;
if (args.includes("--download-cli")) {
  const idx = args.indexOf("--download-cli");
  const url = args[idx + 1];
  const dest = args[idx + 2];
  if (!url || !dest) {
    console.error("Usage: downloader.ts --download-cli <url> <path>");
    Deno.exit(1);
  }
  try {
    const bytes = await downloadFile(url, dest);
    console.log(`OK:${bytes}`);
    Deno.exit(0);
  } catch (e) {
    console.error(`ERROR:${(e as Error).message}`);
    Deno.exit(1);
  }
}

// ── MCP server mode ──

interface DownloadResult {
  success: boolean;
  bytes: number;
  path: string;
  method: string;
  error?: string;
}

createMcpServer({
  name: "downloader",
  version: "1.0.0",
  tools: [
    {
      name: "download_file",
      description:
        "Скачивает файл по URL с цепочкой фоллбэков (reqwest → PowerShell → WinINet). " +
        "Использует системный прокси Windows автоматически.",
      inputSchema: {
        type: "object",
        properties: {
          url: { type: "string", description: "URL для скачивания" },
          output_path: {
            type: "string",
            description: "Путь для сохранения файла",
          },
        },
        required: ["url", "output_path"],
      },
    },
  ],
  handlers: {
    download_file: async (
      args: Record<string, any>,
    ): Promise<string> => {
      const { url, output_path } = args;
      if (!url || !output_path) {
        return JSON.stringify({
          success: false,
          error: "url и output_path обязательны",
        } satisfies DownloadResult);
      }
      try {
        const bytes = await downloadFile(url, output_path);
        const result: DownloadResult = {
          success: true,
          bytes,
          path: output_path,
          method: "powershell_wininet",
        };
        return JSON.stringify(result);
      } catch (e) {
        const result: DownloadResult = {
          success: false,
          bytes: 0,
          path: output_path,
          method: "none",
          error: (e as Error).message,
        };
        return JSON.stringify(result);
      }
    },
  },
});

// ══════════════════════════════════════════════════════════════════════════════
// Скачивание с фоллбэками (PowerShell → WinINet через FFI)
// ══════════════════════════════════════════════════════════════════════════════

async function downloadFile(url: string, dest: string): Promise<number> {
  // Уровень 1: PowerShell Invoke-WebRequest (.NET WebClient + Schannel)
  try {
    const bytes = await downloadViaPowerShell(url, dest);
    log(`[downloader] PowerShell: OK, ${bytes} bytes`);
    return bytes;
  } catch (e) {
    log(`[downloader] PowerShell failed: ${(e as Error).message}`);
  }

  // Уровень 2: WinINet URLDownloadToFile (Deno FFI)
  try {
    const bytes = await downloadViaWinInet(url, dest);
    log(`[downloader] WinINet: OK, ${bytes} bytes`);
    return bytes;
  } catch (e) {
    log(`[downloader] WinINet failed: ${(e as Error).message}`);
  }

  throw new Error("Все методы скачивания не сработали");
}

// ── PowerShell Invoke-WebRequest ──

async function downloadViaPowerShell(
  url: string,
  dest: string,
): Promise<number> {
  const psScript = [
    "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12",
    "$ProgressPreference = 'SilentlyContinue'",
    `Invoke-WebRequest -Uri '${url.replace(/'/g, "''")}' -OutFile '${dest.replace(/'/g, "''")}' -UseBasicParsing`,
    `(Get-Item '${dest.replace(/'/g, "''")}').Length`,
  ].join("; ");

  const cmd = new Deno.Command("powershell", {
    args: [
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
      psScript,
    ],
    stdout: "piped",
    stderr: "piped",
  });

  const output = await cmd.output();
  if (!output.success) {
    const stderr = new TextDecoder().decode(output.stderr);
    throw new Error(`PowerShell: ${stderr.trim()}`);
  }

  const stdout = new TextDecoder().decode(output.stdout).trim();
  const bytes = parseInt(stdout, 10);
  if (isNaN(bytes) || bytes <= 0) {
    // PowerShell скачал, но не вернул размер — читаем из fs
    const stat = await Deno.stat(dest);
    return stat.size;
  }
  return bytes;
}

// ── WinINet URLDownloadToFile через Deno FFI ──

async function downloadViaWinInet(
  url: string,
  dest: string,
): Promise<number> {
  const isWindows = Deno.build.os === "windows";
  if (!isWindows) {
    throw new Error("WinINet доступен только на Windows");
  }

  // urlmon.dll содержит URLDownloadToFileW
  const urlmon = Deno.dlopen("urlmon.dll", {
    URLDownloadToFileW: {
      parameters: [
        "pointer", // pCaller (null)
        "pointer", // szURL
        "pointer", // szFileName
        "u32",     // dwReserved (0)
        "pointer", // lpfnCB (null)
      ],
      result: "i32", // HRESULT
    },
  });

  try {
    const encoder = new TextEncoder();
    const urlBuf = encoder.encode(url + "\0");
    const destBuf = encoder.encode(dest + "\0");

    const urlPtr = Deno.UnsafePointer.of(urlBuf);
    const destPtr = Deno.UnsafePointer.of(destBuf);

    // HRESULT S_OK = 0
    const hr = urlmon.symbols.URLDownloadToFileW(
      null,
      urlPtr,
      destPtr,
      0,
      null,
    );

    if (hr !== 0) {
      throw new Error(`URLDownloadToFileW failed: HRESULT 0x${hr.toString(16)}`);
    }

    const stat = await Deno.stat(dest);
    return stat.size;
  } finally {
    urlmon.close();
  }
}
