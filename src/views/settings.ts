import { aiStatus, setSetting, type AiStatus } from "../api";
import { escapeHtml, friendlyError } from "./util";

export function renderSettings(host: HTMLElement) {
  host.innerHTML = `
    <div class="settings">
      <h1>Settings</h1>
      <div id="ai-status"><p class="hint">Checking local AI…</p></div>
    </div>
  `;
  const box = host.querySelector<HTMLElement>("#ai-status")!;

  function modelOptions(models: string[], selected: string): string {
    const known = models.includes(selected) ? models : [selected, ...models];
    return known
      .map(
        (m) => `<option value="${escapeHtml(m)}" ${m === selected ? "selected" : ""}>${escapeHtml(m)}</option>`,
      )
      .join("");
  }

  function paint(s: AiStatus) {
    box.innerHTML = `
      <p class="ai-dot ${s.running ? "ok" : "bad"}">
        Local AI server ${s.running ? "running" : "not reachable"}
      </p>
      <label>
        Chat &amp; draft model
        <select id="chat-model">${modelOptions(s.models, s.chat_model)}</select>
      </label>
      <label>
        Triage model (faster, lighter)
        <select id="triage-model">${modelOptions(s.models, s.triage_model)}</select>
      </label>
      <p class="hint">On a 16&nbsp;GB Mac, keep the chat model around <code>qwen3:8b-q4_K_M</code>
        (~6–7&nbsp;GB in use) and the triage model at <code>qwen3:1.7b</code> (~2.5&nbsp;GB).
        Pull more with <code>ollama pull &lt;name&gt;</code> in a terminal.</p>
      <p class="status" id="settings-status"></p>
    `;

    const status = box.querySelector<HTMLElement>("#settings-status")!;
    const bind = (id: string, key: string) => {
      box.querySelector<HTMLSelectElement>(`#${id}`)!.addEventListener("change", async (e) => {
        const value = (e.target as HTMLSelectElement).value;
        try {
          await setSetting(key, value);
          status.textContent = "Saved.";
          status.className = "status";
        } catch (err) {
          status.textContent = friendlyError(String(err));
          status.className = "status error";
        }
      });
    };
    bind("chat-model", "chat_model");
    bind("triage-model", "triage_model");
  }

  aiStatus()
    .then(paint)
    .catch((err) => {
      box.innerHTML = `<p class="status error">${escapeHtml(friendlyError(String(err)))}</p>`;
    });
}
