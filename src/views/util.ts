export function escapeHtml(s: string): string {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}

export function friendlyError(raw: string): string {
  const lower = raw.toLowerCase();
  if (lower.includes("authenticationfailed") || lower.includes("invalid credentials")) {
    return "That email/app-password combination didn't work. Double check you generated an app password (not your regular password) and that two-factor authentication is turned on for this account.";
  }
  if (lower.includes("could not reach") || lower.includes("tls handshake")) {
    return "Couldn't reach the mail server. Check your internet connection and that the server address is correct.";
  }
  if (lower.includes("local ai") || lower.includes("ollama")) {
    return `${raw} — make sure the local model is installed (\`ollama pull\`).`;
  }
  return raw;
}

export function debounce<T extends (...args: any[]) => void>(fn: T, ms: number): T {
  let timer: number | undefined;
  return ((...args: any[]) => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => fn(...args), ms);
  }) as T;
}

/** Parse a JSON string array stored in the DB; tolerate nulls and bad data. */
export function parseList(raw: string | null): string[] {
  if (!raw) return [];
  try {
    const v = JSON.parse(raw);
    return Array.isArray(v) ? v.map(String) : [];
  } catch {
    return [];
  }
}
