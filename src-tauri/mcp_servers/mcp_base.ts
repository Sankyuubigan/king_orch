// mcp_base.ts — микро-фреймворк для MCP серверов (Deno).
// Убирает дублирование JSON-RPC протокола (stdin, initialize, tools/list, tools/call).
// Ответы — в stdout (строгий JSON-RPC), логи — в stderr (не ломают протокол).
//
// Использование:
// createMcpServer({
//   name: "my-server",
//   version: "1.0.0",
//   tools: [ { name: "MyTool", description: "...", inputSchema: {...} } ],
//   handlers: {
//     MyTool: (args) => { return "результат"; }
//   }
// });

const encoder = new TextEncoder();
const decoder = new TextDecoder();

// Страховка: в Deno node-compat сбой внутри HTTP-потока (например, EOF при обрыве
// соединения) может вылететь как unhandled rejection и убить весь MCP-процесс.
// Ловим глобально: логируем в stderr (не ломает JSON-RPC) и продолжаем работать.
import process from "node:process";
process.on("unhandledRejection", (reason) => {
  try { Deno.stderr.writeSync(encoder.encode(`[mcp_base] unhandledRejection: ${String(reason)}\n`)); } catch { /* ignore */ }
});

// Буферизованный читатель stdin (построчно).
const BUF_SIZE = 8192;
let stdinBuf = new Uint8Array(BUF_SIZE);
let stdinPos = 0;
let stdinLen = 0;

async function readLine(): Promise<string | null> {
  const lineBytes: number[] = [];
  while (true) {
    if (stdinPos >= stdinLen) {
      const n = await Deno.stdin.read(stdinBuf);
      if (n === null || n === 0) {
        if (lineBytes.length > 0) break;
        return null; // EOF — завершаем цикл
      }
      stdinLen = n;
      stdinPos = 0;
    }
    const byte = stdinBuf[stdinPos++];
    if (byte === 10) break; // \n
    if (byte !== 13) lineBytes.push(byte); // пропускаем \r
  }
  return decoder.decode(new Uint8Array(lineBytes));
}

function sendResponse(id: unknown, result: unknown): void {
  const msg = JSON.stringify({ jsonrpc: "2.0", id, result });
  Deno.stdout.writeSync(encoder.encode(msg + "\n"));
}

export function log(msg: string): void {
  Deno.stderr.writeSync(encoder.encode(msg + "\n"));
}

export interface McpTool {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
}

export interface McpServerConfig {
  name: string;
  version: string;
  tools: McpTool[];
  handlers: Record<string, (args: Record<string, any>) => unknown | Promise<unknown>>;
}

interface JsonRpcRequest {
  id?: unknown;
  method: string;
  params?: { name?: string; arguments?: Record<string, any> };
}

async function handleRequest(req: JsonRpcRequest, config: McpServerConfig): Promise<void> {
  const { id, method, params } = req;
  const { name, version, tools, handlers } = config;

  if (method === "initialize") {
    sendResponse(id, {
      protocolVersion: "2024-11-05",
      capabilities: { tools: {} },
      serverInfo: { name, version },
    });
  } else if (method === "tools/list") {
    sendResponse(id, { tools });
  } else if (method === "tools/call") {
    const toolName = params?.name;
    const args = params?.arguments || {};
    if (toolName && handlers[toolName]) {
      try {
        const result = await handlers[toolName](args);
        sendResponse(id, { content: [{ type: "text", text: String(result) }] });
      } catch (e) {
        sendResponse(id, {
          content: [{ type: "text", text: `Ошибка выполнения: ${(e as Error).message}` }],
        });
      }
    } else {
      sendResponse(id, { content: [{ type: "text", text: `Неизвестный инструмент: ${toolName}` }] });
    }
  }
}

export async function createMcpServer(config: McpServerConfig): Promise<void> {
  while (true) {
    const rawLine = await readLine();
    if (rawLine === null) break;
    if (!rawLine.trim()) continue;
    try {
      const req = JSON.parse(rawLine) as JsonRpcRequest;
      await handleRequest(req, config);
    } catch (e) {
      log(`Ошибка парсинга JSON-RPC: ${(e as Error).message}`);
    }
  }
}
