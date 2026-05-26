const https = require('https');
const { createMcpServer } = require('./mcp_base');

createMcpServer({
    name: "ddg-search-mcp",
    version: "1.0.0",
    tools: [{
        name: "WebSearch",
        description: "Поиск в интернете через DuckDuckGo (быстро, без API ключей)",
        inputSchema: { type: "object", properties: { query: { type: "string", description: "Поисковый запрос" } }, required: ["query"] }
    }],
    handlers: {
        WebSearch: (args) => {
            return new Promise((resolve, reject) => {
                fetchSearch(args.query, (err, html) => {
                    if (err) { reject(new Error(err)); return; }
                    const results = parseResults(html);
                    let textOutput = results.map((r, i) => `[${i+1}] ${r.title}\nСсылка: ${r.url}\nОписание: ${r.snippet}\n`).join('\n');
                    resolve(results.length === 0 ? "Ничего не найдено." : textOutput);
                });
            });
        }
    }
});

function fetchSearch(query, callback) {
    const options = {
        hostname: 'html.duckduckgo.com',
        path: '/html/?q=' + encodeURIComponent(query),
        method: 'GET',
        headers: { 'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36' }
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
    const results = [];
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
        results.push({ title: unescapeHtml(title), url, snippet: unescapeHtml(snippet) });
        if (results.length >= 8) break;
    }
    return results;
}

function unescapeHtml(str) {
    return str.replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&#39;/g, "'");
}