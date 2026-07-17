const https = require('https');
const { createMcpServer } = require('./mcp_base.cjs');

createMcpServer({
    name: "ddg-search-mcp",
    version: "1.1.0",
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
    const postData = 'q=' + encodeURIComponent(query);
    const options = {
        hostname: 'lite.duckduckgo.com',
        path: '/lite/',
        method: 'POST',
        headers: {
            'Content-Type': 'application/x-www-form-urlencoded',
            'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36',
            'Accept': 'text/html',
            'Referer': 'https://lite.duckduckgo.com/'
        }
    };
    const req = https.request(options, (res) => {
        let data = '';
        res.on('data', (chunk) => { data += chunk; });
        res.on('end', () => { callback(null, data); });
    });
    req.on('error', (err) => { callback(err.message, null); });
    req.write(postData);
    req.end();
}

function unescapeHtml(str) {
    return str.replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&#39;/g, "'");
}

function parseResults(html) {
    const results = [];
    const linkRe = /<a\s+[^>]*class='result-link'[^>]*href="([^"]+)"[^>]*>([\s\S]*?)<\/a>|<a\s+[^>]*href="([^"]+)"[^>]*class='result-link'[^>]*>([\s\S]*?)<\/a>/g;
    let m;
    while ((m = linkRe.exec(html)) !== null) {
        let url = m[1] || m[3];
        if (url.startsWith('//')) url = 'https:' + url;
        const title = unescapeHtml((m[2] || m[4]).replace(/<[^>]+>/g, '').trim());

        const after = html.slice(m.index + m[0].length);
        const snipMatch = after.match(/<td class='result-snippet'>([\s\S]*?)<\/td>/);
        const snippet = snipMatch ? unescapeHtml(snipMatch[1].replace(/<[^>]+>/g, '').trim()) : "";

        results.push({ title, url, snippet });
        if (results.length >= 8) break;
    }
    return results;
}
