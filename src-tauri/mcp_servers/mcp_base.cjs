const readline = require('readline');

/**
 * Микро-фреймворк для MCP серверов.
 * Убирает дублирование JSON-RPC протокола (readline, initialize, tools/list, tools/call).
 * 
 * Использование:
 * createMcpServer({
 *   name: "my-server",
 *   version: "1.0.0",
 *   tools: [ { name: "MyTool", description: "...", inputSchema: {...} } ],
 *   handlers: {
 *     MyTool: (args) => { return "результат"; }
 *   }
 * });
 */
function createMcpServer(config) {
    const { name, version, tools, handlers } = config;

    const rl = readline.createInterface({
        input: process.stdin,
        output: process.stdout,
        terminal: false
    });

    function sendResponse(id, result) {
        console.log(JSON.stringify({
            jsonrpc: "2.0",
            id: id,
            result: result
        }));
    }

    rl.on('line', async (line) => {
        try {
            const req = JSON.parse(line);
            const { id, method, params } = req;

            if (method === 'initialize') {
                sendResponse(id, {
                    protocolVersion: "2024-11-05",
                    capabilities: { tools: {} },
                    serverInfo: { name: name, version: version }
                });
            } else if (method === 'tools/list') {
                sendResponse(id, { tools: tools });
            } else if (method === 'tools/call') {
                const toolName = params.name;
                const args = params.arguments || {};

                if (handlers[toolName]) {
                    try {
                        const result = await handlers[toolName](args);
                        sendResponse(id, {
                            content: [{ type: "text", text: String(result) }]
                        });
                    } catch (e) {
                        sendResponse(id, {
                            content: [{ type: "text", text: `Ошибка выполнения: ${e.message}` }]
                        });
                    }
                } else {
                    sendResponse(id, {
                        content: [{ type: "text", text: `Неизвестный инструмент: ${toolName}` }]
                    });
                }
            }
        } catch (e) {
            // Игнорируем битый JSON
        }
    });
}

module.exports = { createMcpServer };