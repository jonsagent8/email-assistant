import { listAccounts, type AccountInfo } from "./api";
import { renderChat } from "./views/chat";
import { renderDrafts } from "./views/drafts";
import { renderInbox } from "./views/inbox";
import { renderSettings } from "./views/settings";
import { renderSetup } from "./views/setup";

const app = document.querySelector<HTMLElement>("#app")!;

type ViewName = "inbox" | "chat" | "drafts" | "settings";

const NAV: { name: ViewName; label: string }[] = [
  { name: "inbox", label: "Inbox" },
  { name: "chat", label: "Assistant" },
  { name: "drafts", label: "Drafts" },
  { name: "settings", label: "Settings" },
];

let accounts: AccountInfo[] = [];
let activeAccountId: number | null = null;
let currentView: ViewName = "inbox";

function accountLabel(): string {
  const a = accounts.find((x) => x.id === activeAccountId);
  return a?.display_name ?? a?.email ?? "Inbox";
}

function renderShell() {
  app.innerHTML = `
    <nav class="nav">
      ${NAV.map(
        (n) =>
          `<button type="button" data-view="${n.name}" class="${
            n.name === currentView ? "active" : ""
          }">${n.label}</button>`,
      ).join("")}
    </nav>
    <div id="view"></div>
  `;
  app.querySelectorAll<HTMLButtonElement>("nav button").forEach((b) => {
    b.addEventListener("click", () => switchView(b.dataset.view as ViewName));
  });
  renderCurrentView();
}

function renderCurrentView() {
  const viewHost = app.querySelector<HTMLElement>("#view")!;
  app.querySelectorAll<HTMLButtonElement>("nav button").forEach((b) => {
    b.classList.toggle("active", b.dataset.view === currentView);
  });

  if (activeAccountId === null) {
    renderSetup(viewHost, (account) => {
      accounts = [...accounts, account];
      activeAccountId = account.id;
      currentView = "inbox";
      renderShell();
    });
    return;
  }

  switch (currentView) {
    case "inbox":
      renderInbox(viewHost, activeAccountId, accountLabel());
      break;
    case "chat":
      renderChat(viewHost);
      break;
    case "drafts":
      renderDrafts(viewHost);
      break;
    case "settings":
      renderSettings(viewHost);
      break;
  }
}

function switchView(view: ViewName) {
  currentView = view;
  renderCurrentView();
}

window.addEventListener("navigate", (e) => {
  const detail = (e as CustomEvent).detail as ViewName;
  if (NAV.some((n) => n.name === detail)) switchView(detail);
});

async function main() {
  accounts = await listAccounts();
  if (accounts.length > 0) {
    activeAccountId = accounts[0].id;
  }
  renderShell();
}

window.addEventListener("DOMContentLoaded", main);
