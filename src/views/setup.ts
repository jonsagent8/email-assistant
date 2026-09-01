import { openUrl } from "@tauri-apps/plugin-opener";
import { addAccount, detectProvider, type AccountInfo, type ProviderInfo } from "../api";
import { debounce, friendlyError } from "./util";

/** Renders the "connect a mailbox" form. Calls `onConnected` with the new account. */
export function renderSetup(host: HTMLElement, onConnected: (account: AccountInfo) => void) {
  host.innerHTML = `
    <div class="setup">
      <h1>Connect your mailbox</h1>
      <p class="hint">Your email and password stay on this machine — nothing is sent anywhere except directly to your email provider and your local AI model.</p>
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

  const emailEl = host.querySelector<HTMLInputElement>("#email")!;
  const providerBox = host.querySelector<HTMLElement>("#provider-box")!;
  const manualFields = host.querySelector<HTMLElement>("#manual-fields")!;
  const imapHostEl = host.querySelector<HTMLInputElement>("#imap_host")!;
  const imapPortEl = host.querySelector<HTMLInputElement>("#imap_port")!;
  const smtpHostEl = host.querySelector<HTMLInputElement>("#smtp_host")!;
  const smtpPortEl = host.querySelector<HTMLInputElement>("#smtp_port")!;
  const statusEl = host.querySelector<HTMLElement>("#status")!;
  const submitBtn = host.querySelector<HTMLButtonElement>("#submit-btn")!;

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
      providerBox
        .querySelector("#get-app-password")
        ?.addEventListener("click", () => openUrl(currentProvider!.app_password_url));
    } else {
      providerBox.classList.add("hidden");
      manualFields.classList.remove("hidden");
    }
  }, 300);

  emailEl.addEventListener("input", refreshProvider);

  host.querySelector("#setup-form")!.addEventListener("submit", async (e) => {
    e.preventDefault();
    submitBtn.disabled = true;
    statusEl.textContent = "Testing connection…";
    statusEl.className = "status";

    try {
      const account = await addAccount({
        email: emailEl.value.trim(),
        password: host.querySelector<HTMLInputElement>("#password")!.value,
        imapHost: imapHostEl.value.trim(),
        imapPort: Number(imapPortEl.value),
        smtpHost: smtpHostEl.value.trim(),
        smtpPort: Number(smtpPortEl.value),
      });
      onConnected(account);
    } catch (err) {
      statusEl.textContent = friendlyError(String(err));
      statusEl.className = "status error";
      submitBtn.disabled = false;
    }
  });
}
