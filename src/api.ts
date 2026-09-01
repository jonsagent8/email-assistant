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
  category: string | null;
  /** JSON-encoded string arrays, or null before triage. */
  labels: string | null;
  action_items: string | null;
}

export interface AssistantReply {
  text: string;
  actions: string[];
}

export interface DraftInfo {
  id: number;
  email_id: number;
  to: string;
  subject: string;
  draft_text: string;
  status: string;
  created_at: string;
  original_from: string;
  original_subject: string;
}

export interface AiStatus {
  running: boolean;
  models: string[];
  chat_model: string;
  triage_model: string;
}

// ---------- accounts ----------

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

// ---------- inbox ----------

export function getCachedEmails(accountId: number): Promise<EmailInfo[]> {
  return invoke("get_cached_emails", { accountId });
}

export function syncInbox(accountId: number): Promise<EmailInfo[]> {
  return invoke("sync_inbox", { accountId });
}

export function triageInbox(accountId: number, limit?: number): Promise<EmailInfo[]> {
  return invoke("triage_inbox", { accountId, limit: limit ?? null });
}

export function summarizeEmail(emailId: number): Promise<string> {
  return invoke("summarize_email", { emailId });
}

export function getEmailFull(emailId: number): Promise<string> {
  return invoke("get_email_full", { emailId });
}

// ---------- assistant ----------

export function assistantChat(sessionId: string, message: string): Promise<AssistantReply> {
  return invoke("assistant_chat", { sessionId, message });
}

// ---------- drafts ----------

export function generateDraft(emailId: number, instructions?: string): Promise<DraftInfo> {
  return invoke("generate_draft", { emailId, instructions: instructions ?? null });
}

export function listDrafts(): Promise<DraftInfo[]> {
  return invoke("list_drafts");
}

export function updateDraft(draftId: number, text: string): Promise<void> {
  return invoke("update_draft", { draftId, text });
}

export function discardDraft(draftId: number): Promise<void> {
  return invoke("discard_draft", { draftId });
}

export function sendDraft(draftId: number): Promise<void> {
  return invoke("send_draft", { draftId });
}

// ---------- settings ----------

export function getSetting(key: string): Promise<string | null> {
  return invoke("get_setting", { key });
}

export function setSetting(key: string, value: string): Promise<void> {
  return invoke("set_setting", { key, value });
}

export function aiStatus(): Promise<AiStatus> {
  return invoke("ai_status");
}
