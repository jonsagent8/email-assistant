import { discardDraft, listDrafts, sendDraft, updateDraft, type DraftInfo } from "../api";
import { escapeHtml, friendlyError } from "./util";

export function renderDrafts(host: HTMLElement) {
  host.innerHTML = `
    <div class="drafts">
      <h1>Drafts</h1>
      <p class="hint">Every reply is written by the local model and waits here. Nothing is sent until you click <strong>Send</strong> and confirm.</p>
      <div id="drafts-list"><p class="empty">Loading…</p></div>
    </div>
  `;
  const listEl = host.querySelector<HTMLElement>("#drafts-list")!;

  function paint(drafts: DraftInfo[]) {
    if (drafts.length === 0) {
      listEl.innerHTML = `<p class="empty">No drafts waiting.</p>`;
      return;
    }
    listEl.innerHTML = drafts
      .map(
        (d) => `
        <div class="draft-card" data-id="${d.id}">
          <div class="draft-meta">
            <div><span class="draft-label">To</span> ${escapeHtml(d.to)}</div>
            <div><span class="draft-label">Subject</span> ${escapeHtml(d.subject)}</div>
          </div>
          <textarea class="draft-body" rows="8">${escapeHtml(d.draft_text)}</textarea>
          <div class="draft-actions">
            <button type="button" data-act="send">Send</button>
            <button type="button" data-act="discard" class="secondary">Discard</button>
            <span class="row-status"></span>
          </div>
        </div>`,
      )
      .join("");
  }

  listEl.addEventListener("click", async (ev) => {
    const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>("button[data-act]");
    if (!btn) return;
    const card = btn.closest<HTMLElement>(".draft-card")!;
    const id = Number(card.dataset.id);
    const bodyEl = card.querySelector<HTMLTextAreaElement>(".draft-body")!;
    const status = card.querySelector<HTMLElement>(".row-status")!;
    const buttons = card.querySelectorAll("button");

    if (btn.dataset.act === "discard") {
      buttons.forEach((b) => (b.disabled = true));
      try {
        await discardDraft(id);
        card.remove();
        if (!listEl.querySelector(".draft-card")) paint([]);
      } catch (err) {
        status.textContent = friendlyError(String(err));
        buttons.forEach((b) => (b.disabled = false));
      }
      return;
    }

    // Send: persist any edits, confirm with the exact content, then send.
    const to = card.querySelector(".draft-meta")!.textContent?.trim() ?? "";
    if (!confirm(`Send this reply?\n\n${to}\n\n${bodyEl.value}`)) return;

    buttons.forEach((b) => (b.disabled = true));
    status.textContent = "Sending…";
    try {
      await updateDraft(id, bodyEl.value);
      await sendDraft(id);
      status.textContent = "Sent ✓";
      card.classList.add("sent");
      bodyEl.disabled = true;
      setTimeout(() => {
        card.remove();
        if (!listEl.querySelector(".draft-card")) paint([]);
      }, 1200);
    } catch (err) {
      status.textContent = friendlyError(String(err));
      buttons.forEach((b) => (b.disabled = false));
    }
  });

  listDrafts()
    .then(paint)
    .catch((err) => {
      listEl.innerHTML = `<p class="empty">${escapeHtml(friendlyError(String(err)))}</p>`;
    });
}
