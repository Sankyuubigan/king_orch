#!/usr/bin/env node
/**
 * gen_docs.cjs — генератор документации из кода King Orch.
 *
 * Убивает «дрейф» ручной docs/ARCHITECTURE.md: строит карту модулей и каталоги
 * инструментов/конфига прямо из исходников. Сгенерированные файлы помечены
 * «НЕ ПРАВИТЬ РУКАМИ» — перегенерируй через `node scripts/gen_docs.cjs`.
 *
 * Соответствует правилу global_ai_docs/core «Архитектура должна быть AI-friendly»
 * (п. 2.7): агент может прочитать актуальную структуру без чтения всего кода.
 */
const fs = require("fs");
const path = require("path");

const ROOT = path.resolve(__dirname, "..");
const SRC = path.join(ROOT, "src-tauri", "src");
const DOCS = path.join(ROOT, "docs");
const MCP_DIR = path.join(ROOT, "src-tauri", "mcp_servers");

const GENERATED_HEADER = (title) =>
  `# ${title} (сгенерировано автоматически)\n\n` +
  "> ⚠️ ФАЙЛ СГЕНЕРИРОВАН скриптом `scripts/gen_docs.cjs`. **НЕ ПРАВИТЬ РУКАМИ** —\n" +
  "> правки будут перезаписаны при следующей генерации. Правь код, затем перегенерируй.\n\n";

function walk(dir, ext, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full, ext, out);
    else if (entry.name.endsWith(ext)) out.push(full);
  }
  return out;
}

// Превращает путь к .rs-файлу в crate-путь (crate::a::b).
function fileToCratePath(file) {
  let rel = path.relative(SRC, file).replace(/\\/g, "/");
  rel = rel.replace(/\.rs$/, "");
  if (rel.endsWith("/mod")) rel = rel.slice(0, -4);
  return "crate::" + rel.split("/").filter(Boolean).join("::");
}

// ---- 1. Карта модулей (mermaid) ----
function buildModuleGraph() {
  const files = walk(SRC, ".rs");
  const cratePaths = new Set(files.map(fileToCratePath));
  const edges = [];
  const nodes = new Set();
  for (const file of files) {
    const src = fs.readFileSync(file, "utf8");
    const from = fileToCratePath(file);
    nodes.add(from);
    // use crate::a::b::c;  (поддержка групповых и одиночных)
    const re = /use\s+crate::([\w:]+(?:::\*)?)\s*;/g;
    let m;
    while ((m = re.exec(src))) {
      const target = m[1].replace(/::\*$/, "");
      if (cratePaths.has(target) && target !== from) {
        edges.push([from, target]);
        nodes.add(target);
      }
    }
  }
  const id = (p) => p.replace(/[^A-Za-z0-9_]/g, "_");
  let md = "## Карта модулей (зависимости через `use crate::`)\n\n```mermaid\nflowchart TD\n";
  for (const n of nodes) {
    md += `  ${id(n)}["${n}"]\n`;
  }
  for (const [a, b] of edges) {
    md += `  ${id(a)} --> ${id(b)}\n`;
  }
  md += "```\n\n";
  return md;
}

// ---- 2. Каталог инструментов ----
function buildToolCatalog() {
  let md = "## Built-in инструменты (код)\n\n";
  const runtime = path.join(SRC, "domain", "orchestrator", "runtime.rs");
  if (fs.existsSync(runtime)) {
    const src = fs.readFileSync(runtime, "utf8");
    const re = /"name":\s*"(\w+)"[\s\S]*?description:\s*"([^"]*)"/g;
    let m;
    while ((m = re.exec(src))) {
      md += `- \`${m[1]}\` — ${m[2]}\n`;
    }
  }
  md += "\n## MCP-серверы (Deno, src-tauri/mcp_servers/)\n\n";
  const servers = walk(MCP_DIR, ".ts").map((f) => path.basename(f, ".ts")).sort();
  if (servers.length === 0) {
    md += "_MCP-серверы не найдены._\n";
  } else {
    for (const s of servers) {
      const f = path.join(MCP_DIR, s + ".ts");
      const src = fs.readFileSync(f, "utf8");
      const names = new Set();
      let m;
      const tr = /name:\s*"([\w_-]+)"/g;
      while ((m = tr.exec(src))) names.add(m[1]);
      const toolList = [...names].sort().join("`, `") || "_(инструменты не распознаны статически)_";
      md += `- **${s}** — инструменты: \`${toolList}\`\n`;
    }
  }
  md += "\n";
  return md;
}

// ---- 3. Каталог конфига ----
function buildConfigCatalog() {
  const cfg = path.join(SRC, "infra", "config.rs");
  let md = "## Структуры конфигурации (infra/config.rs)\n\n";
  if (!fs.existsSync(cfg)) {
    md += "_config.rs не найден._\n";
    return md;
  }
  const src = fs.readFileSync(cfg, "utf8");
  for (const structName of ["AppConfig", "ModelParams"]) {
    const start = src.indexOf(`pub struct ${structName}`);
    if (start < 0) continue;
    const end = src.indexOf("}", start);
    const block = src.slice(start, end);
    md += `### ${structName}\n\n`;
    const re = /pub\s+(\w+)\s*:\s*([\w<>,\s]+?)\s*(?:,|=)/g;
    let m;
    while ((m = re.exec(block))) {
      md += `- \`${m[1]}\`: \`${m[2].trim()}\`\n`;
    }
    md += "\n";
  }
  return md;
}

function main() {
  if (!fs.existsSync(DOCS)) fs.mkdirSync(DOCS, { recursive: true });
  const moduleGraph = GENERATED_HEADER("Карта модулей King Orch") + buildModuleGraph();
  const toolCatalog = GENERATED_HEADER("Каталог инструментов King Orch") + buildToolCatalog();
  const configCatalog = GENERATED_HEADER("Каталог конфигурации King Orch") + buildConfigCatalog();

  fs.writeFileSync(path.join(DOCS, "GENERATED_MODULE_GRAPH.md"), moduleGraph);
  fs.writeFileSync(path.join(DOCS, "GENERATED_TOOL_CATALOG.md"), toolCatalog);
  fs.writeFileSync(path.join(DOCS, "GENERATED_CONFIG_CATALOG.md"), configCatalog);
  console.log("✅ Сгенерировано:");
  console.log("   docs/GENERATED_MODULE_GRAPH.md");
  console.log("   docs/GENERATED_TOOL_CATALOG.md");
  console.log("   docs/GENERATED_CONFIG_CATALOG.md");
}

main();
