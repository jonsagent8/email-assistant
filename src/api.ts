import { invoke } from "@tauri-apps/api/core";

export interface ProviderInfo {
  imap_host: string;
  imap_port: number;
  smtp_host: string;
  smtp_port: number;
  app_password_url: string;
}

export interface AccountInfo {
  id: number;
  email: string;
  display_name: string | null;
  imap_host: string;
  imap_port: number;
  smtp_host: string;
  smtp_port: number;
}

export interface EmailInfo {
  id: number;
  uid: number;
  from_addr: string | null;
  to_addr: string | null;
  subject: string | null;
  date: string | null;
  snippet: string;
  is_read: boolean;
}

export function detectProvider(email: string): Promise<ProviderInfo | null> {
  return invoke("detect_provider", { email });
}

export function addAccount(params: {
  email: string;
  password: string;
  imapHost: string;
  imapPort: number;
  smtpHost: string;
  smtpPort: number;
  displayName?: string | null;
}): Promise<AccountInfo> {
  return invoke("add_account", {
    email: params.email,
    password: params.password,
    imapHost: params.imapHost,
    imapPort: params.imapPort,
    smtpHost: params.smtpHost,
    smtpPort: params.smtpPort,
    displayName: params.displayName ?? null,
  });
}

export function listAccounts(): Promise<AccountInfo[]> {
  return invoke("list_accounts");
}

export function getCachedEmails(accountId: number): Promise<EmailInfo[]> {
  return invoke("get_cached_emails", { accountId });
}

export function syncInbox(accountId: number): Promise<EmailInfo[]> {
  return invoke("sync_inbox", { accountId });
}
