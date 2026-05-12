const https = require('https');
const readline = require('readline');

const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
    terminal: false
});

rl.on('line', (line) => {
    try {
        const req = JSON.parse(line);
        handleRequest(req);
    } catch (e) {}
});

function sendResponse(id, result) {
    console.log(JSON.stringify({
        jsonrpc: "2.0",
        id: id,
        result: result
    }));
}

function handleRequest(req) {
    const { id, method, params } = req;
    if (method === 'initialize') {
        sendResponse(id, {
            protocolVersion: "2024-11-05",
            capabilities: { tools: {} },
            serverInfo: { name: "docs-fetcher-mcp", version: "1.0.0" }
        });
    } else if (method === 'tools/list') {
        sendResponse(id, {
            tools:[
                {
                    name: "WebFetch",
                    description: "Скачать веб-страницу или документацию по ссылке и извлечь чистый текст для анализа",
                    inputSchema: {
                        type: "object",
                        properties: {
                            url: { type: "string", description: "URL адрес страницы" }
                        },
                        required: ["url"]
                    }
                }
            ]
        });
    } else if (method === 'tools/call') {
        const { name, arguments: args } = params;
        if (name === 'WebFetch') {
            fetchUrl(args.url, (err, html) => {
                if (err) {
                    sendResponse(id, { content:[{ type: "text", text: `Ошибка загрузки страницы: ${err}` }] });
                } else {
                    const cleanText = cleanHtml(html);
                    sendResponse(id, { content:[{ type: "text", text: cleanText }] });
                }
            });
        }
    }
}

function fetchUrl(targetUrl, callback) {
    try {
        const urlObj = new URL(targetUrl);
        const options = {
            hostname: urlObj.hostname,
            path: urlObj.pathname + urlObj.search,
            method: 'GET',
            headers: {
                'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
                'Accept': 'text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8'
            }
        };
        const client = urlObj.protocol === 'https:' ? https : require('http');
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
    } catch (e) {
        callback(e.message, null);
    }
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
    return str
        .replace(/&amp;/g, '&')
        .replace(/&lt;/g, '<')
        .replace(/&gt;/g, '>')
        .replace(/&quot;/g, '"')
        .replace(/&#x27;/g, "'")
        .replace(/&#39;/g, "'");
}