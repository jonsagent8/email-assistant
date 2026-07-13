import { openUrl } from "@tauri-apps/plugin-opener";
import {
  addAccount,
  detectProvider,
  getCachedEmails,
  listAccounts,
  syncInbox,
  type AccountInfo,
  type EmailInfo,
  type ProviderInfo,
} from "./api";

const app = document.querySelector<HTMLElement>("#app")!;

let accounts: AccountInfo[] = [];
let activeAccountId: number | null = null;

async function main() {
  accounts = await listAccounts();
  if (accounts.length === 0) {
    renderSetupView();
  } else {
    activeAccountId = accounts[0].id;
    await renderInboxView();
  }
}

// ---------- Account setup ----------

function renderSetupView() {
  app.innerHTML = `
    <div class="setup">
      <h1>Connect your mailbox</h1>
      <p class="hint">Your email and password stay on this machine — nothing is sent anywhere except directly to your email provider.</p>
      <form id="setup-form">
        <label>
          Email address
          <input id="email" type="email" required autocomplete="email" placeholder="you@gmail.com" />
        </label>

        <div id="provider-box" class="provider-box hidden"></div>

        <div id="manual-fields" class="manual-fields hidden">
          <label>IMAP host <input id="imap_host" type="text" /></label>
          <label>IMAP port <input id="imap_port" type="number" value="993" /></label>
          <label>SMTP host <input id="smtp_host" type="text" /></label>
          <label>SMTP port <input id="smtp_port" type="number" value="587" /></label>
        </div>

        <label>
          App password
          <input id="password" type="password" required autocomplete="current-password" placeholder="paste the app password here" />
        </label>

        <button type="submit" id="submit-btn">Connect</button>
        <p id="status" class="status"></p>
      </form>
    </div>
  `;

  const emailEl = document.querySelector<HTMLInputElement>("#email")!;
  const providerBox = document.querySelector<HTMLElement>("#provider-box")!;
  const manualFields = document.querySelector<HTMLElement>("#manual-fields")!;
  const imapHostEl = document.querySelector<HTMLInputElement>("#imap_host")!;
  const imapPortEl = document.querySelector<HTMLInputElement>("#imap_port")!;
  const smtpHostEl = document.querySelector<HTMLInputElement>("#smtp_host")!;
  const smtpPortEl = document.querySelector<HTMLInputElement>("#smtp_port")!;
  const statusEl = document.querySelector<HTMLElement>("#status")!;
  const submitBtn = document.querySelector<HTMLButtonElement>("#submit-btn")!;

  let currentProvider: ProviderInfo | null = null;

  const refreshProvider = debounce(async () => {
    const email = emailEl.value.trim();
    if (!email.includes("@")) {
      providerBox.classList.add("hidden");
      manualFields.classList.add("hidden");
      return;
    }
    currentProvider = await detectProvider(email);
    if (currentProvider) {
      manualFields.classList.add("hidden");
      const providerName = currentProvider.imap_host.replace(/^imap\./, "");
      providerBox.classList.remove("hidden");
      providerBox.innerHTML = `
        <p>Detected <strong>${providerName}</strong> settings automatically.</p>
        <button type="button" id="get-app-password">Get my app password ↗</button>
      `;
      imapHostEl.value = currentProvider.imap_host;
      imapPortEl.value = String(currentProvider.imap_port);
      smtpHostEl.value = currentProvider.smtp_host;
      smtpPortEl.value = String(currentProvider.smtp_port);
      document
        .querySelector("#get-app-password")
        ?.addEventListener("click", () => openUrl(currentProvider!.app_password_url));
    } else {
      providerBox.classList.add("hidden");
      manualFields.classList.remove("hidden");
    }
  }, 300);

  emailEl.addEventListener("input", refreshProvider);

  document.querySelector("#setup-form")!.addEventListener("submit", async (e) => {
    e.preventDefault();
    submitBtn.disabled = true;
    statusEl.textContent = "Testing connection…";
    statusEl.className = "status";

    try {
      const account = await addAccount({
        email: emailEl.value.trim(),
        password: document.querySelector<HTMLInputElement>("#password")!.value,
        imapHost: imapHostEl.value.trim(),
        imapPort: Number(imapPortEl.value),
        smtpHost: smtpHostEl.value.trim(),
        smtpPort: Number(smtpPortEl.value),
      });
      accounts = [...accounts, account];
      activeAccountId = account.id;
      await renderInboxView();
    } catch (err) {
      statusEl.textContent = friendlyError(String(err));
      statusEl.className = "status error";
      submitBtn.disabled = false;
    }
  });
}

function friendlyError(raw: string): string {
  const lower = raw.toLowerCase();
  if (lower.includes("authenticationfailed") || lower.includes("invalid credentials")) {
    return "That email/app-password combination didn't work. Double check you generated an app password (not your regular password) and that two-factor authentication is turned on for this account.";
  }
  if (lower.includes("could not reach") || lower.includes("tls handshake")) {
    return "Couldn't reach the mail server. Check your internet connection and that the server address is correct.";
  }
  return raw;
}

function debounce<T extends (...args: any[]) => void>(fn: T, ms: number): T {
  let timer: number | undefined;
  return ((...args: any[]) => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => fn(...args), ms);
  }) as T;
}

// ---------- Inbox ----------

async function renderInboxView() {
  const account = accounts.find((a) => a.id === activeAccountId)!;
  const cached = await getCachedEmails(account.id);
  paintInbox(account, cached, true);

  try {
    const fresh = await syncInbox(account.id);
    paintInbox(account, fresh, false);
  } catch (err) {
    const banner = document.querySelector<HTMLElement>("#sync-banner");
    if (banner) {
      banner.textContent = `Sync failed: ${friendlyError(String(err))}`;
      banner.classList.remove("hidden");
    }
  }
}

function paintInbox(account: AccountInfo, emails: EmailInfo[], syncing: boolean) {
  app.innerHTML = `
    <div class="inbox">
      <header>
        <h1>${account.display_name ?? account.email}</h1>
        <span class="sync-state">${syncing ? "Syncing…" : `${emails.length} messages`}</span>
      </header>
      <p id="sync-banner" class="banner hidden"></p>
      <ul class="email-list">
        ${emails
          .map(
            (e) => `
          <li>
            <div class="email-from">${escapeHtml(e.from_addr ?? "(unknown sender)")}</div>
            <div class="email-subject">${escapeHtml(e.subject ?? "(no subject)")}</div>
            <div class="email-snippet">${escapeHtml(e.snippet)}</div>
          </li>`,
          )
          .join("")}
      </ul>
      ${emails.length === 0 && !syncing ? '<p class="empty">No messages yet.</p>' : ""}
    </div>
  `;
}

function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}

window.addEventListener("DOMContentLoaded", main);
