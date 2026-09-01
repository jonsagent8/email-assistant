import {
  generateDraft,
  getCachedEmails,
  summarizeEmail,
  syncInbox,
  triageInbox,
  type EmailInfo,
} from "../api";
import { escapeHtml, friendlyError, parseList } from "./util";

const CATEGORY_LABEL: Record<string, string> = {
  urgent: "Urgent",
  needs_reply: "Needs reply",
  fyi: "FYI",
  newsletter: "Newsletter",
  spam: "Spam",
};

function navigate(view: string) {
  window.dispatchEvent(new CustomEvent("navigate", { detail: view }));
}

export function renderInbox(host: HTMLElement, accountId: number, accountLabel: string) {
  host.innerHTML = `
    <div class="inbox">
      <header>
        <h1>${escapeHtml(accountLabel)}</h1>
        <div class="inbox-actions">
          <button type="button" id="triage-btn" class="secondary">Triage</button>
          <span class="sync-state" id="sync-state">Syncing…</span>
        </div>
      </header>
      <p id="sync-banner" class="banner hidden"></p>
      <ul class="email-list" id="email-list"></ul>
      <p class="empty hidden" id="empty">No messages yet.</p>
    </div>
  `;

  const listEl = host.querySelector<HTMLElement>("#email-list")!;
  const emptyEl = host.querySelector<HTMLElement>("#empty")!;
  const syncStateEl = host.querySelector<HTMLElement>("#sync-state")!;
  const bannerEl = host.querySelector<HTMLElement>("#sync-banner")!;
  const triageBtn = host.querySelector<HTMLButtonElement>("#triage-btn")!;

  function paint(emails: EmailInfo[]) {
    emptyEl.classList.toggle("hidden", emails.length > 0);
    listEl.innerHTML = emails
      .map((e) => {
        const actions = parseList(e.action_items);
        const cat = e.category
          ? `<span class="badge badge-${e.category}">${CATEGORY_LABEL[e.category] ?? e.category}</span>`
          : "";
        const actionsHtml = actions.length
          ? `<ul class="action-items">${actions
              .map((a) => `<li>${escapeHtml(a)}</li>`)
              .join("")}</ul>`
          : "";
        return `
          <li data-id="${e.id}" data-subject="${escapeHtml(e.subject ?? "")}">
            <div class="email-row-top">
              <span class="email-from">${escapeHtml(e.from_addr ?? "(unknown sender)")}</span>
              ${cat}
            </div>
            <div class="email-subject">${escapeHtml(e.subject ?? "(no subject)")}</div>
            <div class="email-snippet">${escapeHtml(e.snippet)}</div>
            ${actionsHtml}
            <div class="email-actions">
              <button type="button" data-act="summarize">Summarize</button>
              <button type="button" data-act="draft">Draft reply</button>
              <span class="row-status"></span>
            </div>
            <div class="summary hidden"></div>
          </li>`;
      })
      .join("");
  }

  listEl.addEventListener("click", async (ev) => {
    const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>("button[data-act]");
    if (!btn) return;
    const li = btn.closest<HTMLElement>("li")!;
    const id = Number(li.dataset.id);
    const rowStatus = li.querySelector<HTMLElement>(".row-status")!;
    const summaryEl = li.querySelector<HTMLElement>(".summary")!;
    li.querySelectorAll("button").forEach((b) => (b.disabled = true));

    try {
      if (btn.dataset.act === "summarize") {
        rowStatus.textContent = "Summarizing…";
        const s = await summarizeEmail(id);
        summaryEl.textContent = s;
        summaryEl.classList.remove("hidden");
        rowStatus.textContent = "";
      } else {
        rowStatus.textContent = "Drafting…";
        await generateDraft(id);
        rowStatus.textContent = "Draft ready →";
        rowStatus.style.cursor = "pointer";
        rowStatus.onclick = () => navigate("drafts");
      }
    } catch (err) {
      rowStatus.textContent = friendlyError(String(err));
    } finally {
      li.querySelectorAll("button").forEach((b) => (b.disabled = false));
    }
  });

  triageBtn.addEventListener("click", async () => {
    triageBtn.disabled = true;
    triageBtn.textContent = "Triaging…";
    try {
      paint(await triageInbox(accountId));
    } catch (err) {
      bannerEl.textContent = friendlyError(String(err));
      bannerEl.classList.remove("hidden");
    } finally {
      triageBtn.disabled = false;
      triageBtn.textContent = "Triage";
    }
  });

  // Show cache immediately, then sync in the background.
  getCachedEmails(accountId).then(paint).catch(() => {});
  syncInbox(accountId)
    .then((fresh) => {
      paint(fresh);
      syncStateEl.textContent = `${fresh.length} messages`;
    })
    .catch((err) => {
      syncStateEl.textContent = "";
      bannerEl.textContent = `Sync failed: ${friendlyError(String(err))}`;
      bannerEl.classList.remove("hidden");
    });
}
