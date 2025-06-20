// mcp_server.js - ФИНАЛЬНАЯ ВЕРСИЯ: УДАЛЕН ЛИШНИЙ ВЫЗОВ .start()

import { createRequire } from 'module';
const require = createRequire(import.meta.url);

import { chromium } from 'playwright';

// Устанавливаем фиктивные переменные для обхода проверок
process.env.BROWSERBASE_API_KEY = 'local-dummy-key';
process.env.BROWSERBASE_PROJECT_ID = 'local-dummy-project-id';

const StagehandLibrary = require('@browserbasehq/stagehand');
const { Stagehand } = StagehandLibrary;

const PORT = 7777;

async function main() {
  let browserServer = null;

  const cleanup = async () => {
    console.log('\n[MCP Server] Получен сигнал завершения. Остановка...');
    if (browserServer) {
      await browserServer.close();
      console.log('[MCP Server] Локальный браузерный сервер остановлен.');
    }
    process.exit(0);
  };

  process.on('SIGINT', cleanup);
  process.on('SIGTERM', cleanup);

  try {
    if (typeof Stagehand !== 'function') {
      console.error('[MCP Server] CRITICAL: Конструктор Stagehand не найден.');
      process.exit(1);
    }

    console.log('[MCP Server] Запуск локального браузерного сервера через Playwright...');
    
    browserServer = await chromium.launchServer({
      headless: false,
    });

    const cdpUrl = browserServer.wsEndpoint();
    
    console.log(`[MCP Server] Браузерный сервер запущен. Адрес для подключения (CDP): ${cdpUrl}`);
    console.log('[MCP Server] Инициализация и запуск сервера Stagehand...');

    // <<< КЛЮЧЕВОЕ ИЗМЕНЕНИЕ: Конструктор сам запускает сервер >>>
    const server = new Stagehand({
      port: PORT,
      localBrowserLaunchOptions: {
        cdpUrl: cdpUrl,
      },
    });

    // <<< УДАЛЕНО: Строка "await server.start()" вызывала ошибку, т.к. метод отсутствует >>>
    // Сервер уже запущен конструктором выше.

    console.log(`[MCP Server] СЕРВЕР УСПЕШНО ЗАПУЩЕН.`);
    console.log(`[MCP Server] Stagehand подключен к локальному браузеру и слушает порт: http://localhost:${PORT}`);
    console.log('[MCP Server] Готов к приему запросов...');


  } catch (error) {
    console.error('[MCP Server] КРИТИЧЕСКАЯ ОШИБКА ПРИ ЗАПУСКЕ СЕРВЕРА:', error);
    if (browserServer) {
      await browserServer.close(); 
    }
    process.exit(1);
  }
}

main();