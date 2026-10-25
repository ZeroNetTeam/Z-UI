import type { NotifyEvents, NotifyPayload, NotifyRelief, NotifySettingsView } from "@/lib/api";

/** What the notifications page edits; the secrets start empty and "set" says whether one is stored. */
export interface NotifyForm {
  language: "fa" | "en";
  proxy: string;
  telegramEnabled: boolean;
  token: string;
  tokenSet: boolean;
  apiBase: string;
  /** One per line, or separated by commas. */
  chatIds: string;
  userBot: boolean;
  notifyUsers: boolean;
  discordEnabled: boolean;
  discordUrl: string;
  discordUrlSet: boolean;
  webhookEnabled: boolean;
  webhookUrl: string;
  webhookSecret: string;
  webhookSecretSet: boolean;
  events: NotifyEvents;
  relief: NotifyRelief;
}

export function fromView(view: NotifySettingsView): NotifyForm {
  return {
    language: view.language,
    proxy: view.proxy,
    telegramEnabled: view.telegram.enabled,
    token: "",
    tokenSet: view.telegram.token_set,
    apiBase: view.telegram.api_base,
    chatIds: view.telegram.chat_ids.join("\n"),
    userBot: view.telegram.user_bot,
    notifyUsers: view.telegram.notify_users,
    discordEnabled: view.discord.enabled,
    discordUrl: "",
    discordUrlSet: view.discord.url_set,
    webhookEnabled: view.webhook.enabled,
    webhookUrl: view.webhook.url,
    webhookSecret: "",
    webhookSecretSet: view.webhook.secret_set,
    events: { ...view.events },
    relief: { ...view.relief },
  };
}

/** Chat ids from text (a line or a comma each): the good ones, and the words that are not ids. */
export function parseChatIds(text: string): { ids: number[]; bad: string[] } {
  const ids: number[] = [];
  const bad: string[] = [];
  for (const word of text.split(/[\s,;]+/).filter(Boolean)) {
    if (/^-?\d{1,18}$/.test(word) && Number(word) !== 0) ids.push(Number(word));
    else bad.push(word);
  }
  return { ids: [...new Set(ids)], bad };
}

/**
 * What is sent. The backup settings go back as they are: the server replaces
 * the whole of the settings, and the page that edits them is another one.
 */
export function toPayload(form: NotifyForm, view: NotifySettingsView): NotifyPayload {
  const { passphrase_set: _set, ...backup } = view.backup;
  void _set;
  return {
    language: form.language,
    proxy: form.proxy.trim(),
    telegram: {
      enabled: form.telegramEnabled,
      token: form.token.trim(),
      api_base: form.apiBase.trim(),
      chat_ids: parseChatIds(form.chatIds).ids,
      user_bot: form.userBot,
      notify_users: form.notifyUsers,
    },
    discord: { enabled: form.discordEnabled, url: form.discordUrl.trim() },
    webhook: { enabled: form.webhookEnabled, url: form.webhookUrl.trim(), secret: form.webhookSecret.trim() },
    events: { ...form.events },
    relief: { ...form.relief },
    backup: { ...backup, passphrase: "" },
  };
}

/** Whether the form differs from what is stored: any typed secret does. */
export function isDirty(form: NotifyForm, view: NotifySettingsView): boolean {
  const stored = fromView(view);
  const normal = (f: NotifyForm) => ({
    ...f,
    proxy: f.proxy.trim(),
    apiBase: f.apiBase.trim(),
    chatIds: parseChatIds(f.chatIds).ids.sort((a, b) => a - b).join(","),
    webhookUrl: f.webhookUrl.trim(),
  });
  return JSON.stringify(normal(form)) !== JSON.stringify(normal(stored));
}
