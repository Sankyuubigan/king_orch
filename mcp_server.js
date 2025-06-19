// mcp_server.js - ФИНАЛЬНАЯ ВЕРСИЯ ПОСЛЕ УСТАНОВКИ С GITHUB

// Этот импорт будет работать, т.к. npm создаст ссылку на пакет @stagehand/agent
// внутри папки node_modules после установки с GitHub.
import { Agent, MCP } from '@stagehand/agent';

async function main() {
  try {
    const agent = new Agent({
      localBrowserLaunchOptions: {
        headless: false, // Запускаем браузер в видимом режиме
      }
    });

    const mcp = new MCP(agent);
    mcp.listen();
    
    console.log('[MCP Server] Сервер запущен. При получении команды будет запущен видимый браузер.');

  } catch (error) {
    console.error('[MCP Server] Критическая ошибка при запуске:', error);
    process.exit(1);
  }
}

main();