import { confirmDialog } from "./confirm";

export interface MessageMenuCallbacks {
  onDelete: (msgUid: string) => void;
  onClone: (msgUid: string) => void;
}

let activeMenu: HTMLDivElement | null = null;

document.addEventListener("click", (e) => {
  if (activeMenu && !activeMenu.parentElement?.contains(e.target as Node)) {
    activeMenu.classList.remove("show");
    activeMenu = null;
  }
});

export function createMessageMenu(msgUid: string, callbacks: MessageMenuCallbacks): HTMLDivElement {
  const wrapper = document.createElement("div");
  wrapper.className = "msg-menu-wrapper";

  const btn = document.createElement("button");
  btn.className = "msg-menu-btn";
  btn.innerHTML = "⋮";
  btn.title = "Действия";

  const dropdown = document.createElement("div");
  dropdown.className = "msg-menu-dropdown";

  const deleteItem = document.createElement("button");
  deleteItem.className = "msg-menu-item danger";
  deleteItem.textContent = "🗑️ Удалить";
  deleteItem.addEventListener("click", async (e) => {
    e.stopPropagation();
    dropdown.classList.remove("show");
    activeMenu = null;
    const confirmed = await confirmDialog(
      "Удаление сообщения",
      "Вы уверены, что хотите удалить это сообщение из сессии?"
    );
    if (confirmed) {
      callbacks.onDelete(msgUid);
    }
  });

  const cloneItem = document.createElement("button");
  cloneItem.className = "msg-menu-item";
  cloneItem.textContent = "📋 Создать Клон сессии";
  cloneItem.addEventListener("click", (e) => {
    e.stopPropagation();
    dropdown.classList.remove("show");
    activeMenu = null;
    callbacks.onClone(msgUid);
  });

  dropdown.appendChild(deleteItem);
  dropdown.appendChild(cloneItem);
  wrapper.appendChild(btn);
  wrapper.appendChild(dropdown);

  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    e.preventDefault();
    if (activeMenu && activeMenu !== dropdown) {
      activeMenu.classList.remove("show");
    }
    dropdown.classList.toggle("show");
    activeMenu = dropdown.classList.contains("show") ? dropdown : null;
  });

  return wrapper;
}