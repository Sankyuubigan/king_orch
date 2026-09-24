export interface WorkspaceMenuAction {
  id: string;
  label: string;
  icon: string;
  action: () => void | Promise<void>;
  separatorBefore?: boolean;
}

export interface WorkspaceMenuController {
  element: HTMLElement;
  open: (focusFirst?: boolean) => void;
  close: (restoreFocus?: boolean) => void;
}

export function createWorkspaceMenu(
  anchor: HTMLButtonElement,
  actions: WorkspaceMenuAction[],
): WorkspaceMenuController {
  const menu = document.createElement("div");
  menu.className = "workspace-tab-menu-dropdown";
  menu.setAttribute("role", "menu");
  menu.setAttribute("aria-label", "Главное меню");
  menu.id = anchor.getAttribute("aria-controls") || `workspace-menu-${Date.now().toString(36)}`;

  for (const action of actions) {
    if (action.separatorBefore) {
      const separator = document.createElement("div");
      separator.className = "workspace-tab-menu-separator";
      separator.setAttribute("role", "separator");
      menu.appendChild(separator);
    }
    const item = document.createElement("button");
    item.className = "workspace-tab-menu-item";
    item.type = "button";
    item.dataset.actionId = action.id;
    item.setAttribute("role", "menuitem");
    const icon = document.createElement("span");
    icon.className = "menu-ico";
    icon.textContent = action.icon;
    const label = document.createElement("span");
    label.textContent = action.label;
    item.append(icon, label);
    item.addEventListener("click", () => {
      close(false);
      void action.action();
    });
    menu.appendChild(item);
  }

  const items = () => Array.from(menu.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'));
  let isOpen = false;
  let restoreFocusOnClose = false;

  const position = () => {
    const margin = 8;
    const gap = 6;
    const rect = anchor.getBoundingClientRect();
    const width = Math.min(300, window.innerWidth - margin * 2);
    const height = menu.getBoundingClientRect().height;
    const below = window.innerHeight - rect.bottom - gap - margin;
    const above = rect.top - gap - margin;
    const opensAbove = below < height && above > below;
    menu.classList.toggle("opens-above", opensAbove);
    menu.style.left = `${Math.min(Math.max(margin, rect.right - width), window.innerWidth - width - margin)}px`;
    menu.style.top = `${opensAbove ? Math.max(margin, rect.top - height - gap) : Math.min(window.innerHeight - height - margin, rect.bottom + gap)}px`;
  };

  const onPointerDown = (event: PointerEvent) => {
    const target = event.target as Node;
    if (!menu.contains(target) && !anchor.contains(target)) close(false);
  };

  const onKeydown = (event: KeyboardEvent) => {
    const menuItems = items();
    if (event.key === "Escape") {
      event.preventDefault();
      close(true);
      return;
    }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const current = menuItems.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "Home" ? 0
      : event.key === "End" ? menuItems.length - 1
      : event.key === "ArrowDown" ? (current + 1 + menuItems.length) % menuItems.length
      : (current - 1 + menuItems.length) % menuItems.length;
    menuItems[next]?.focus();
  };

  function open(focusFirst = false) {
    if (isOpen) return;
    isOpen = true;
    restoreFocusOnClose = focusFirst;
    menu.style.width = `${Math.min(300, window.innerWidth - 16)}px`;
    document.body.appendChild(menu);
    position();
    anchor.setAttribute("aria-expanded", "true");
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeydown, true);
    window.addEventListener("resize", position);
    window.addEventListener("scroll", position, true);
    window.visualViewport?.addEventListener("resize", position);
    window.visualViewport?.addEventListener("scroll", position);
    if (focusFirst) items()[0]?.focus();
  }

  function close(restoreFocus = false) {
    if (!isOpen) return;
    isOpen = false;
    menu.remove();
    anchor.setAttribute("aria-expanded", "false");
    document.removeEventListener("pointerdown", onPointerDown, true);
    document.removeEventListener("keydown", onKeydown, true);
    window.removeEventListener("resize", position);
    window.removeEventListener("scroll", position, true);
    window.visualViewport?.removeEventListener("resize", position);
    window.visualViewport?.removeEventListener("scroll", position);
    if (restoreFocus || restoreFocusOnClose) anchor.focus();
    restoreFocusOnClose = false;
  }

  return { element: menu, open, close };
}
