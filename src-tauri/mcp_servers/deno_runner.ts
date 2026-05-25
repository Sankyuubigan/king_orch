// deno_runner.ts — MCP Server: изолированная песочница Deno
// Запускает .ts/.js файлы с жёсткими ограничениями:
// - Чтение/запись только в .agents_workspace/sandbox/
// - Нет сети, нет подпроцессов, нет env, нет sys

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function log(msg: string): void {
  Deno.stderr.writeSync(encoder.encode(`[DENO-RUNNER] ${msg}\n`));
}

log("=== Deno Sandbox Runner MCP v1.0 ===");

// ============================================
// PATH UTILITIES (без доступа к FS)
// ============================================

function normalizePath(p: string): string {
  return p.replace(/\\/g, "/").replace(/\/+/g, "/").replace(/\/$/, "");
}

function resolvePath(...segments: string[]): string {
  const combined = segments.join("/");
  const parts = normalizePath(combined).split("/");
  const resolved: string[] = [];
  for (const part of parts) {
    if (part === "..") {
      if (resolved.length > 0) resolved.pop();
    } else if (part !== "." && part !== "") {
      resolved.push(part);
    }
  }
  return resolved.join("/");
}

function isInsideDir(filePath: string, dir: string): boolean {
  const normFile = normalizePath(filePath);
  const normDir = normalizePath(dir);
  return normFile.startsWith(normDir + "/") || normFile === normDir;
}

// ============================================
// STDIN READER (буферизованный)
// ============================================

const BUF_SIZE = 8192;
let stdinBuf = new Uint8Array(BUF_SIZE);
let stdinPos = 0;
let stdinLen = 0;

async function readLine(): Promise<string> {
  const lineBytes: number[] = [];
  while (true) {
    if (stdinPos >= stdinLen) {
      const n = await Deno.stdin.read(stdinBuf);
      if (n === null || n === 0) {
        if (lineBytes.length > 0) break;
        Deno.exit(0);
      }
      stdinLen = n;
      stdinPos = 0;
    }
    const byte = stdinBuf[stdinPos++];
    if (byte === 10) break; // \n
    if (byte !== 13) lineBytes.push(byte); // skip \r
  }
  return decoder.decode(new Uint8Array(lineBytes));
}

// ============================================
// MCP PROTOCOL
// ============================================

function send(id: number | null, result: unknown): void {
  const msg = JSON.stringify({ jsonrpc: "2.0", id, result });
  Deno.stdout.writeSync(encoder.encode(msg + "\n"));
}

async function readStream(stream: ReadableStream<Uint8Array>): Promise<string> {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
  }
  const totalLen = chunks.reduce((s, c) => s + c.length, 0);
  const combined = new Uint8Array(totalLen);
  let off = 0;
  for (const c of chunks) {
    combined.set(c, off);
    off += c.length;
  }
  return decoder.decode(combined);
}

// ============================================
// SANDBOX EXECUTION
// ============================================

async function runSandbox(
  filePath: string,
  projectPath: string,
  timeoutSec: number,
): Promise<string> {
  // Резолвим пути и проверяем безопасность
  const sandboxDir = resolvePath(projectPath, ".agents_workspace", "sandbox");
  const resolvedFile = resolvePath(filePath);

  if (!isInsideDir(resolvedFile, sandboxDir)) {
    return `❌ ОШИБКА БЕЗОПАСНОСТИ: Файл должен находиться в директории песочницы:\n   ${sandboxDir}\n   Получен путь: ${filePath}\n   Резолвленный: ${resolvedFile}`;
  }

  // Проверяем расширение
  const ext = resolvedFile.split(".").pop()?.toLowerCase();
  if (ext !== "ts" && ext !== "js" && ext !== "mjs") {
    return `❌ Поддерживаются только файлы .ts, .js, .mjs. Получено: .${ext}`;
  }

  const denoExePath = Deno.execPath();

  log(`Запуск песочницы: ${resolvedFile}`);
  log(`Sandbox dir: ${sandboxDir}`);
  log(`Deno exe: ${denoExePath}`);
  log(`Таймаут: ${timeoutSec}с`);

  // Спавним дочерний процесс с жёсткими ограничениями:
  // --allow-read=<sandbox>  — чтение только из песочницы
  // --allow-write=<sandbox> — запись только в песочницу
  // --no-check              — без проверки типов (быстрее)
  // --no-config             — без чтения deno.json
  // НЕТ --allow-net, --allow-run, --allow-env, --allow-sys
  const command = new Deno.Command(denoExePath, {
    args: [
      "run",
      `--allow-read=${sandboxDir}`,
      `--allow-write=${sandboxDir}`,
      "--no-check",
      "--no-config",
      resolvedFile,
    ],
    stdout: "piped",
    stderr: "piped",
  });

  let child;
  try {
    child = command.spawn();
  } catch (e) {
    return `❌ Не удалось запустить процесс Deno: ${e}`;
  }

  // Таймаут
  let timedOut = false;
  const timeoutId = setTimeout(() => {
    timedOut = true;
    try {
      child.kill();
      log(`⏱️ Процесс убит по таймауту (${timeoutSec}с)`);
    } catch (e) {
      log(`Ошибка при убийстве процесса: ${e}`);
    }
  }, timeoutSec * 1000);

  // Ждём завершения
  let status;
  try {
    status = await child.status;
  } catch (e) {
    clearTimeout(timeoutId);
    return `❌ Ошибка ожидания процесса: ${e}`;
  }
  clearTimeout(timeoutId);

  // Читаем выходные потоки
  const [stdout, stderr] = await Promise.all([
    readStream(child.stdout),
    readStream(child.stderr),
  ]);

  // Формируем результат
  let result = "";

  if (timedOut) {
    result += `⏱️ ТАЙМАУТ: скрипт не завершился за ${timeoutSec} сек и был принудительно остановлен.\n\n`;
  }

  if (stdout.trim()) {
    result += `--- STDOUT ---\n${stdout}\n`;
  }
  if (stderr.trim()) {
    result += `--- STDERR ---\n${stderr}\n`;
  }

  result += `--- Код завершения: ${status.code} ---`;

  if (status.code !== 0 && !timedOut) {
    result = `⚠️ Скрипт завершился с ошибкой (код: ${status.code})\n\n${result}`;
  } else if (status.code === 0 && !timedOut) {
    result = `✅ Скрипт выполнен успешно\n\n${result}`;
  }

  return result;
}

// ============================================
// REQUEST HANDLER
// ============================================

async function handleRequest(req: {
  id?: number;
  method: string;
  params?: { name?: string; arguments?: Record<string, unknown> };
}): Promise<void> {
  const { id, method, params } = req;

  if (method === "initialize") {
    send(id ?? null, {
      protocolVersion: "2024-11-05",
      capabilities: { tools: {} },
      serverInfo: { name: "deno-runner-mcp", version: "1.0.0" },
    });
  } else if (method === "tools/list") {
    send(id ?? null, {
      tools: [{
        name: "run_sandbox",
        description:
          "Запускает TypeScript/JavaScript файл в изолированной песочнице Deno. " +
          "Код выполняется в строгой изоляции: чтение/запись ТОЛЬКО в .agents_workspace/sandbox/, " +
          "без сети, без подпроцессов, без доступа к env. Возвращает stdout, stderr и код завершения. " +
          "НЕ используйте внешние импорты — пишите assert-функции вручную.",
        inputSchema: {
          type: "object",
          properties: {
            file_path: {
              type: "string",
              description:
                "Абсолютный путь к .ts/.js файлу для выполнения (обязательно внутри .agents_workspace/sandbox/)",
            },
            project_path: {
              type: "string",
              description: "Корневой путь проекта (для формирования sandbox-директории)",
            },
            timeout_sec: {
              type: "number",
              description: "Таймаут выполнения в секундах (по умолчанию: 30, максимум: 120)",
            },
          },
          required: ["file_path", "project_path"],
        },
      }],
    });
  } else if (method === "tools/call") {
    if (params?.name === "run_sandbox") {
      const args = params.arguments ?? {};
      const timeout = Math.min(
        Math.max(typeof args.timeout_sec === "number" ? args.timeout_sec : 30, 1),
        120,
      );
      const result = await runSandbox(
        String(args.file_path ?? ""),
        String(args.project_path ?? ""),
        timeout,
      );
      send(id ?? null, { content: [{ type: "text", text: result }] });
    } else {
      send(id ?? null, {
        content: [{ type: "text", text: `Неизвестный инструмент: ${params?.name}` }],
      });
    }
  }
}

// ============================================
// MAIN LOOP
// ============================================

log("✅ Готов к запуску песочниц Deno (безопасная изоляция)");
while (true) {
  try {
    const line = await readLine();
    if (!line.trim()) continue;
    const req = JSON.parse(line);
    await handleRequest(req);
  } catch (e) {
    log(`Error: ${e}`);
  }
}