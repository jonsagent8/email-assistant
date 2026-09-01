import { assistantChat } from "../api";
import { escapeHtml, friendlyError } from "./util";

// One conversation per app launch.
const sessionId = crypto.randomUUID();

interface Turn {
  role: "user" | "assistant";
  text: string;
  actions?: string[];
}

const turns: Turn[] = [];

export function renderChat(host: HTMLElement) {
  host.innerHTML = `
    <div class="chat">
      <h1>Assistant</h1>
      <p class="hint">Ask about your inbox in plain language — "what did Dana last email about?",
        "find everything about the Q3 invoice", "draft a reply to the landlord saying I'll pay Friday".
        It reads your local cache only and never sends anything without you clicking Send in Drafts.</p>
      <div class="chat-log" id="chat-log"></div>
      <form class="chat-input" id="chat-form">
        <textarea id="chat-text" rows="2" placeholder="Ask something…"></textarea>
        <button type="submit" id="chat-send">Send</button>
      </form>
    </div>
  `;

  const logEl = host.querySelector<HTMLElement>("#chat-log")!;
  const formEl = host.querySelector<HTMLFormElement>("#chat-form")!;
  const textEl = host.querySelector<HTMLTextAreaElement>("#chat-text")!;
  const sendBtn = host.querySelector<HTMLButtonElement>("#chat-send")!;

  function paint() {
    logEl.innerHTML = turns
      .map((t) => {
        const actions =
          t.actions && t.actions.length
            ? `<div class="chat-actions">${t.actions.map(escapeHtml).join(" · ")}</div>`
            : "";
        return `
          <div class="chat-turn chat-${t.role}">
            <div class="chat-bubble">${escapeHtml(t.text).replace(/\n/g, "<br>")}</div>
            ${actions}
          </div>`;
      })
      .join("");
    logEl.scrollTop = logEl.scrollHeight;
  }
  paint();

  async function submit() {
    const message = textEl.value.trim();
    if (!message) return;
    textEl.value = "";
    turns.push({ role: "user", text: message });
    turns.push({ role: "assistant", text: "…" });
    paint();
    sendBtn.disabled = true;
    textEl.disabled = true;

    try {
      const reply = await assistantChat(sessionId, message);
      turns[turns.length - 1] = {
        role: "assistant",
        text: reply.text,
        actions: reply.actions,
      };
    } catch (err) {
      turns[turns.length - 1] = { role: "assistant", text: friendlyError(String(err)) };
    } finally {
      sendBtn.disabled = false;
      textEl.disabled = false;
      textEl.focus();
      paint();
    }
  }

  formEl.addEventListener("submit", (e) => {
    e.preventDefault();
    void submit();
  });
  textEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit();
    }
  });
}
