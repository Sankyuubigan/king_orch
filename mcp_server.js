// mcp_server.js - ФИНАЛЬНАЯ ВЕРСИЯ. ВОЗВРАЩАЕМ ВИДИМЫЙ БРАУЗЕР ДЛЯ СТАБИЛЬНОСТИ.

import { createRequire } from 'module';
const require = createRequire(import.meta.url);

import { chromium } from 'playwright';
import express from 'express';

const PORT = 7777;
const HOST = '127.0.0.1';

async function main() {
  let browser = null;
  let page = null;

  const cleanup = async () => {
    console.log('\n[MCP Server] Получен сигнал завершения. Остановка...');
    if (browser && browser.isConnected()) await browser.close();
    console.log('[MCP Server] Браузер остановлен.');
    process.exit(0);
  };

  process.on('SIGINT', cleanup);
  process.on('SIGTERM', cleanup);

  try {
    // <<< ГЛАВНОЕ ИЗМЕНЕНИЕ: ВОЗВРАЩАЕМ ВИДИМЫЙ РЕЖИМ >>>
    // Это единственный надежный способ обойти сетевые блокировки в некоторых системах.
    console.log('[MCP Server] Запуск браузера в ВИДИМОМ режиме для максимальной совместимости...');
    browser = await chromium.launch({ headless: false });
    const context = await browser.newContext();
    page = await context.newPage();
    console.log('[MCP Server] Браузер и страница готовы.');

    const app = express();
    app.use(express.json());

    app.get('/health', (req, res) => {
      res.status(200).send('OK');
    });

    app.get('/screenshot', async (req, res) => {
      if (page && !page.isClosed()) {
        try {
          const buffer = await page.screenshot({ type: 'png' });
          res.set('Content-Type', 'image/png');
          res.send(buffer);
        } catch (e) {
          res.status(500).send('Failed to take screenshot');
        }
      } else {
        res.status(404).send('Page not available');
      }
    });

    app.post('/v1/action', async (req, res) => {
      const goal = req.body?.action?.goal;
      if (!goal) {
        return res.status(400).json({ error: 'Missing goal in request body' });
      }

      console.log(`[ACTION] Получена задача: ${goal}`);
      try {
        await page.goto(`https://www.google.com/search?q=${encodeURIComponent(goal)}`, { waitUntil: 'domcontentloaded', timeout: 60000 });
        await page.waitForTimeout(2000);
        const content = await page.evaluate(() => document.body.innerText);
        res.json({ result: content.substring(0, 2000) });
        console.log(`[ACTION] Задача выполнена, результат отправлен.`);
      } catch (e) {
        console.error(`[ACTION] Ошибка выполнения задачи: ${e}`);
        res.status(500).json({ error: e.message });
      }
    });

    app.listen(PORT, HOST, () => {
      console.log(`[MCP Server] Единый сервер запущен на http://${HOST}:${PORT}`);
      console.log('[MCP Server] MCP_SERVER_READY_FOR_CONNECTIONS');
    });

  } catch (error) {
    console.error('[MCP Server] КРИТИЧЕСКАЯ ОШИБКА ПРИ ЗАПУСКЕ СЕРВЕРА:', error);
    if (browser) await browser.close();
    process.exit(1);
  }
}

main();