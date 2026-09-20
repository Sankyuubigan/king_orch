// ─── Чистые хелперы визуального редактора условий condition_router ───
// Модель данных (зеркало Rust ConditionNode): массив `conditions`, где элемент —
// лист { field, equals } либо группа { logic?, conditions[] }. Корневой массив =
// корневая группа с логикой `data.logic`.

export interface CondRule {
  field: string;
  equals: unknown;
}

export interface CondGroup {
  logic?: string;
  conditions: CondNode[];
}

export type CondNode = CondRule | CondGroup;

export interface CondFactsInfo {
  id: string;
  values?: string[];
}

export interface CondRenderOptions {
  esc: (s: string) => string;
  facts: CondFactsInfo[];
  knownFields: string[];
}

export function isConditionGroup(c: unknown): c is CondGroup {
  return !!c && typeof c === "object" && Array.isArray((c as any).conditions) && !("field" in (c as any));
}

export function isConditionRule(c: unknown): c is CondRule {
  return !!c && typeof c === "object" && "field" in (c as any);
}

export function parseEqualsValue(value: string): unknown {
  const t = value.trim();
  if (t === "true") return true;
  if (t === "false") return false;
  if (t !== "" && !isNaN(Number(t))) return Number(t);
  return t;
}

export function condValueToString(v: unknown): string {
  if (v === true) return "true";
  if (v === false) return "false";
  if (typeof v === "number") return String(v);
  return String(v ?? "");
}

export function conditionAt(root: { conditions: CondNode[] }, path: number[]): CondNode | null {
  let arr: CondNode[] = root.conditions;
  let cur: CondNode | null = null;
  for (let i = 0; i < path.length; i++) {
    if (!Array.isArray(arr) || path[i] >= arr.length) return null;
    cur = arr[path[i]];
    if (i < path.length - 1) {
      if (!isConditionGroup(cur)) return null;
      arr = (cur as CondGroup).conditions;
    }
  }
  return cur;
}

export function conditionSiblings(root: { conditions: CondNode[] }, path: number[]): CondNode[] | null {
  if (path.length < 1) return null;
  if (path.length === 1) return root.conditions;
  const parent = conditionAt(root, path.slice(0, -1));
  return parent && isConditionGroup(parent) ? (parent as CondGroup).conditions : null;
}

// Массив children группы по пути группы (путь [] = корневой массив).
export function conditionGroupList(root: { conditions: CondNode[] }, path: number[]): CondNode[] {
  if (path.length === 0) return root.conditions;
  const g = conditionAt(root, path);
  return g && isConditionGroup(g) ? (g as CondGroup).conditions : [];
}

function isBooleanFact(facts: CondFactsInfo[], field: string): boolean {
  const f = facts.find((x) => x.id === field);
  return !!f && !(f.values && f.values.length > 0);
}

function isEnumFact(facts: CondFactsInfo[], field: string): string[] | null {
  const f = facts.find((x) => x.id === field);
  return f && f.values && f.values.length > 0 ? f.values : null;
}

function optionsHtml(opts: CondRenderOptions, options: string[], current: string): string {
  const set = new Set([...options, current]);
  let html = "";
  set.forEach((o) => {
    html += `<option value="${opts.esc(o)}" ${o === current ? "selected" : ""}>${opts.esc(o)}</option>`;
  });
  return html;
}

export function renderConditionsTreeHtml(data: any, opts: CondRenderOptions): string {
  const root = Array.isArray(data.conditions) ? data.conditions : [];
  const rootLogic = data.logic === "all" ? "all" : "any";
  return renderGroupBlock(root, [], rootLogic, true, opts);
}

function renderGroupBlock(items: CondNode[], path: number[], logic: string, isRoot: boolean, opts: CondRenderOptions): string {
  const pathAttr = JSON.stringify(path);
  let html = `<div class="ge-cond-group${isRoot ? " ge-cond-root" : ""}" data-path="${pathAttr}">`;
  html += `<div class="ge-cond-group-head">
      <span class="ge-cond-badge">${isRoot ? "ВСЕ УСЛОВИЯ" : "ГРУППА"}</span>
      <select class="ge-select ge-cond-logic" data-path="${pathAttr}" data-isroot="${isRoot ? 1 : 0}">
        <option value="any" ${logic !== "all" ? "selected" : ""}>любое (OR)</option>
        <option value="all" ${logic === "all" ? "selected" : ""}>все (AND)</option>
      </select>`;
  if (!isRoot) {
    html += ` <button class="ge-cond-remove ge-cond-remove-group" data-path="${pathAttr}" title="Удалить группу">🗑</button>`;
  }
  html += `</div>`;
  html += `<div class="ge-cond-children">`;
  items.forEach((c, i) => {
    const childPath = [...path, i];
    if (isConditionRule(c)) {
      html += renderRuleRow(c, childPath, path.length, opts);
    } else if (isConditionGroup(c)) {
      html += renderGroupBlock(c.conditions, childPath, c.logic === "all" ? "all" : "any", false, opts);
    }
  });
  html += `</div>`;
  html += `<div class="ge-cond-group-foot">
      <button class="ge-cond-add" data-path="${pathAttr}" title="Добавить условие в эту группу">+ условие</button>
      <button class="ge-cond-add-group" data-path="${pathAttr}" title="Добавить вложенную группу">+ группа</button>
    </div>`;
  html += `</div>`;
  return html;
}

function renderRuleRow(c: CondRule, path: number[], depth: number, opts: CondRenderOptions): string {
  const pathAttr = JSON.stringify(path);
  const field = c.field ?? "";
  const equalsRaw = condValueToString(c.equals);

  // Логика выбора контрола значения по полю.
  let valueHtml = "";
  let valueHint = "";
  const enumVals = isEnumFact(opts.facts, field);
  if (enumVals) {
    valueHint = "одно из: " + enumVals.join(" | ");
    valueHtml = `<select class="ge-input ge-cond-equals" data-path="${pathAttr}">${optionsHtml(opts, enumVals, equalsRaw)}</select>`;
  } else if (isBooleanFact(opts.facts, field) || (!field.includes(".") && field !== "" && !isEnumFact(opts.facts, field))) {
    valueHint = "true — есть / false — нет";
    valueHtml = `<select class="ge-input ge-cond-equals" data-path="${pathAttr}">${optionsHtml(opts, ["true", "false"], equalsRaw)}</select>`;
  } else {
    valueHtml = `<input class="ge-input ge-cond-equals" data-path="${pathAttr}" value="${opts.esc(equalsRaw)}" placeholder="true / число / строка" />`;
  }

  let html = `<div class="ge-cond-row" data-path="${pathAttr}" style="padding-left:${Math.min(depth * 6, 36)}px;">`;
  html += `<input class="ge-input ge-cond-field" data-path="${pathAttr}" value="${opts.esc(field)}" placeholder="агент, факт или signal.field" list="ge-cond-fields-list" title="${opts.esc(valueHint)}" />`;
  html += `<span class="ge-cond-eq">=</span>`;
  html += valueHtml;
  html += `<button class="ge-cond-up" data-path="${pathAttr}" title="Вверх">⬆</button>`;
  html += `<button class="ge-cond-down" data-path="${pathAttr}" title="Вниз">⬇</button>`;
  html += `<button class="ge-cond-remove" data-path="${pathAttr}" title="Удалить">🗑</button>`;
  html += `</div>`;
  return html;
}

export function buildKnownFields(nodes: any[]): string[] {
  const fields = new Set<string>();
  nodes.forEach((n) => {
    if (n && n.id) fields.add(n.id);
    const conds = n && Array.isArray(n.conditions) ? n.conditions : null;
    if (conds) {
      const collect = (list: CondNode[]) => {
        list.forEach((c) => {
          if (isConditionRule(c)) fields.add(c.field);
          else if (isConditionGroup(c)) collect(c.conditions);
        });
      };
      collect(conds);
    }
  });
  return Array.from(fields);
}

// Сводка на канвасе: рекурсивное выражение со скобками.
export function renderConditionExpression(nodes: CondNode[], logic: string, esc: (s: string) => string, maxLen = 120): string {
  const parts = nodes.map((c) => {
    if (isConditionRule(c)) return `${c.field}=${condValueToString(c.equals)}`;
    return `(${renderConditionExpression((c as CondGroup).conditions, (c as CondGroup).logic || "any", esc, maxLen)})`;
  });
  const sep = logic === "all" ? " AND " : " OR ";
  let expr = parts.join(sep);
  if (expr.length > maxLen) expr = expr.slice(0, maxLen) + "…";
  return esc(expr);
}