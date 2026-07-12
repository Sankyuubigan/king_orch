import { ThoughtMenuCallbacks } from "../types";
import { confirmDialog } from "./confirm";

export function createThoughtsBlock(
  initialItems?: HTMLElement[],
  assistantUid?: string | null,
  menuCallbacks?: ThoughtMenuCallbacks,
  thoughtUids?: string[]
): HTMLDivElement {
  const block = document.createElement("div");
  block.className = "thoughts-block";
  const count = initialItems ? initialItems.length : 0;

  const header = document.createElement("div");
  header.className = "thoughts-header";
  
  const titleSpan = document.createElement("span");
  titleSpan.className = "thoughts-title";
  titleSpan.innerText = `💭 Мысли (${count}) `;
  
  const toggle = document.createElement("span");
  toggle.className = "thoughts-toggle";
  toggle.textContent = "▶";
  header.appendChild(toggle);
  header.appendChild(titleSpan);

  if (menuCallbacks && (assistantUid || (thoughtUids && thoughtUids.length > 0))) {
    const menuWrapper = document.createElement("div");
    menuWrapper.className = "msg-menu-wrapper";
    const btn = document.createElement("button");
    btn.className = "msg-menu-btn";
    btn.innerHTML = "⋮";
    btn.title = "Действия";
    const dropdown = document.createElement("div");
    dropdown.className = "msg-menu-dropdown";
    const deleteItem = document.createElement("button");
    deleteItem.className = "msg-menu-item danger";
    deleteItem.textContent = "🗑️ Удалить мысли";
    deleteItem.addEventListener("click", async (e) => {
      e.stopPropagation();
      dropdown.classList.remove("show");
      const confirmed = await confirmDialog(
        "Удаление мыслей",
        "Вы уверены, что хотите удалить этот блок мыслей из сессии?"
      );
      if (confirmed) {
        menuCallbacks.onDeleteThoughts(assistantUid ?? null, thoughtUids ?? []);
      }
    });
    const cloneItem = document.createElement("button");
    cloneItem.className = "msg-menu-item";
    cloneItem.textContent = "📋 Клон до мыслей";
    cloneItem.addEventListener("click", (e) => { e.stopPropagation(); dropdown.classList.remove("show"); menuCallbacks.onCloneFromThoughts(assistantUid!); });
    dropdown.appendChild(deleteItem);
    if (assistantUid) dropdown.appendChild(cloneItem);
    menuWrapper.appendChild(btn);
    menuWrapper.appendChild(dropdown);
    header.appendChild(menuWrapper);
    btn.addEventListener("click", (e) => { e.stopPropagation(); e.preventDefault(); dropdown.classList.toggle("show"); });
  }

  const content = document.createElement("div");
  content.className = "thoughts-content thoughts-collapsed";
  if (initialItems) initialItems.forEach(item => content.appendChild(item));

  header.addEventListener("click", (e) => {
    if ((e.target as HTMLElement).closest('.msg-menu-wrapper')) return;
    const isCollapsed = content.classList.contains("thoughts-collapsed");
    content.classList.toggle("thoughts-collapsed", !isCollapsed);
    content.classList.toggle("thoughts-expanded", isCollapsed);
    toggle.textContent = isCollapsed ? "▼" : "▶";
  });

  block.appendChild(header);
  block.appendChild(content);
  return block;
}

export function addToThoughtsBlock(block: HTMLDivElement, item: HTMLElement): void {
  const content = block.querySelector(".thoughts-content") as HTMLDivElement;
  const titleSpan = block.querySelector(".thoughts-title") as HTMLSpanElement;
  if (!content || !titleSpan) return;
  content.appendChild(item);
  const count = content.children.length;
  titleSpan.innerText = `💭 Мысли (${count}) `;
}