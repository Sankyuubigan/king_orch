/**
 * Сворачиваемый блок «Мысли» — группирует мысли агентов и отчёты сабагентов.
 * По умолчанию свёрнут. Клик по заголовку переключает видимость.
 */

export function createThoughtsBlock(initialItems?: HTMLElement[]): HTMLDivElement {
  const block = document.createElement("div");
  block.className = "thoughts-block";

  const count = initialItems ? initialItems.length : 0;

  const header = document.createElement("div");
  header.className = "thoughts-header";
  header.innerHTML = `💭 Мысли (${count}) <span class="thoughts-toggle">▶</span>`;

  const content = document.createElement("div");
  content.className = "thoughts-content thoughts-collapsed";
  if (initialItems) {
    initialItems.forEach(item => content.appendChild(item));
  }

  header.addEventListener("click", () => {
    const isCollapsed = content.classList.contains("thoughts-collapsed");
    content.classList.toggle("thoughts-collapsed", !isCollapsed);
    content.classList.toggle("thoughts-expanded", isCollapsed);
    const toggle = header.querySelector(".thoughts-toggle");
    if (toggle) toggle.textContent = isCollapsed ? "▼" : "▶";
  });

  block.appendChild(header);
  block.appendChild(content);

  return block;
}

export function addToThoughtsBlock(block: HTMLDivElement, item: HTMLElement): void {
  const content = block.querySelector(".thoughts-content") as HTMLDivElement;
  const header = block.querySelector(".thoughts-header") as HTMLDivElement;
  if (!content || !header) return;

  content.appendChild(item);

  const count = content.children.length;
  const isCollapsed = content.classList.contains("thoughts-collapsed");
  header.innerHTML = `💭 Мысли (${count}) <span class="thoughts-toggle">${isCollapsed ? "▶" : "▼"}</span>`;
}