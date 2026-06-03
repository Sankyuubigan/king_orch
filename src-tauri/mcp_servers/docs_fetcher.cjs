const https = require('https');
const http = require('http');
const { createMcpServer } = require('./mcp_base.cjs');

createMcpServer({
    name: "docs-fetcher-mcp",
    version: "1.0.0",
    tools: [{
        name: "WebFetch",
        description: "Скачать веб-страницу или документацию по ссылке и извлечь чистый текст",
        inputSchema: { type: "object", properties: { url: { type: "string", description: "URL адрес страницы" } }, required: ["url"] }
    }],
    handlers: {
        WebFetch: (args) => {
            return new Promise((resolve, reject) => {
                fetchUrl(args.url, (err, html) => {
                    if (err) { reject(new Error(err)); return; }
                    resolve(cleanHtml(html));
                });
            });
        }
    }
});

function fetchUrl(targetUrl, callback) {
    try {
        const urlObj = new URL(targetUrl);
        const options = {
            hostname: urlObj.hostname,
            path: urlObj.pathname + urlObj.search,
            method: 'GET',
            headers: { 'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36' }
        };
        const client = urlObj.protocol === 'https:' ? https : http;
        const req = client.request(options, (res) => {
            if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
                return fetchUrl(res.headers.location, callback);
            }
            let data = '';
            res.on('data', (chunk) => { data += chunk; });
            res.on('end', () => { callback(null, data); });
        });
        req.on('error', (err) => { callback(err.message, null); });
        req.end();
    } catch (e) { callback(e.message, null); }
}

function cleanHtml(html) {
    let cleaned = html.replace(/<(script|style|head|iframe|nav|footer|header)[^>]*>[\s\S]*?<\/\1>/gi, '');
    cleaned = cleaned.replace(/<!--[\s\S]*?-->/g, '');
    cleaned = cleaned.replace(/<h1[^>]*>([\s\S]*?)<\/h1>/gi, '\n# $1\n');
    cleaned = cleaned.replace(/<h2[^>]*>([\s\S]*?)<\/h2>/gi, '\n## $1\n');
    cleaned = cleaned.replace(/<h3[^>]*>([\s\S]*?)<\/h3>/gi, '\n### $1\n');
    cleaned = cleaned.replace(/<p[^>]*>([\s\S]*?)<\/p>/gi, '\n$1\n');
    cleaned = cleaned.replace(/<li[^>]*>([\s\S]*?)<\/li>/gi, '\n- $1');
    cleaned = cleaned.replace(/<br\s*\/?>/gi, '\n');
    cleaned = cleaned.replace(/<[^>]+>/g, '');
    cleaned = unescapeHtml(cleaned);
    cleaned = cleaned.split('\n').map(line => line.trim()).filter(line => line.length > 0).join('\n');
    return cleaned;
}

function unescapeHtml(str) {
    return str.replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&#39;/g, "'");
}