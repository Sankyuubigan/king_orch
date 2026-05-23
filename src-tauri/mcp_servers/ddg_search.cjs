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
            serverInfo: { name: "ddg-search-mcp", version: "1.0.0" }
        });
    } else if (method === 'tools/list') {
        sendResponse(id, {
            tools:[
                {
                    name: "WebSearch",
                    description: "Поиск в интернете через DuckDuckGo (быстро, без API ключей)",
                    inputSchema: {
                        type: "object",
                        properties: {
                            query: { type: "string", description: "Поисковый запрос" }
                        },
                        required: ["query"]
                    }
                }
            ]
        });
    } else if (method === 'tools/call') {
        const { name, arguments: args } = params;
        if (name === 'WebSearch') {
            fetchSearch(args.query, (err, html) => {
                if (err) {
                    sendResponse(id, { content:[{ type: "text", text: `Ошибка поиска: ${err}` }] });
                } else {
                    const results = parseResults(html);
                    let textOutput = results.map((r, i) => `[${i+1}] ${r.title}\nСсылка: ${r.url}\nОписание: ${r.snippet}\n`).join('\n');
                    if (results.length === 0) textOutput = "Ничего не найдено.";
                    sendResponse(id, { content: [{ type: "text", text: textOutput }] });
                }
            });
        }
    }
}

function fetchSearch(query, callback) {
    const options = {
        hostname: 'html.duckduckgo.com',
        path: '/html/?q=' + encodeURIComponent(query),
        method: 'GET',
        headers: {
            'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
            'Accept': 'text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8'
        }
    };
    const req = https.request(options, (res) => {
        let data = '';
        res.on('data', (chunk) => { data += chunk; });
        res.on('end', () => { callback(null, data); });
    });
    req.on('error', (err) => { callback(err.message, null); });
    req.end();
}

function parseResults(html) {
    const results =[];
    const blocks = html.split('class="result__body"');
    for (let i = 1; i < blocks.length; i++) {
        const block = blocks[i];
        const aStart = block.indexOf('class="result__a"');
        if (aStart === -1) continue;
        const hrefStart = block.indexOf('href="', aStart);
        if (hrefStart === -1) continue;
        const hrefEnd = block.indexOf('"', hrefStart + 6);
        let url = block.substring(hrefStart + 6, hrefEnd);
        if (url.startsWith('//')) url = 'https:' + url;
        
        const titleStart = block.indexOf('>', hrefEnd);
        if (titleStart === -1) continue;
        const titleEnd = block.indexOf('</a>', titleStart);
        if (titleEnd === -1) continue;
        let title = block.substring(titleStart + 1, titleEnd).replace(/<[^>]+>/g, '').trim();
        
        const snippetClass = 'class="result__snippet"';
        const snippetStart = block.indexOf(snippetClass);
        let snippet = "";
        if (snippetStart !== -1) {
            const tagClose = block.indexOf('>', snippetStart + snippetClass.length);
            const tagEnd = block.indexOf('</a>', tagClose);
            if (tagClose !== -1 && tagEnd !== -1) {
                snippet = block.substring(tagClose + 1, tagEnd).replace(/<[^>]+>/g, '').trim();
            }
        }
        title = unescapeHtml(title);
        snippet = unescapeHtml(snippet);
        results.push({ title, url, snippet });
        if (results.length >= 8) break;
    }
    return results;
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