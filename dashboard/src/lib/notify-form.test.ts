import { describe, expect, it } from "vitest";
import type { NotifySettingsView } from "./api";
import { fromView, isDirty, parseChatIds, toPayload } from "./notify-form";

const view: NotifySettingsView = {
  language: "en",
  proxy: "socks5://bot:•••@10.0.0.1:1080",
  telegram: { enabled: true, token_set: true, api_base: "https://api.telegram.org", chat_ids: [-1001, 5], user_bot: true, notify_users: false },
  discord: { enabled: false, url_set: true },
  webhook: { enabled: true, url: "https://hooks.example/z", secret_set: true },
  events: {
    data_low: true,
    data_low_percent: 85,
    expiring: true,
    expiring_days: 3,
    limited: true,
    expired: false,
    core: true,
    login_failed: true,
    login_new: true,
    backup: true,
  },
  backup: { enabled: true, interval_hours: 12, keep: 5, send_telegram: false, include_usage: true, passphrase_set: true },
};

describe("notify form", () => {
  it("starts with every secret empty and says which are set", () => {
    const form = fromView(view);
    expect(form).toMatchObject({ token: "", tokenSet: true, discordUrl: "", discordUrlSet: true, webhookSecret: "", webhookSecretSet: true });
    expect(form.chatIds).toBe("-1001\n5");
  });

  it("reads chat ids however they are separated and names what is not one", () => {
    expect(parseChatIds("-1001234567890, 42\n42  ;7")).toEqual({ ids: [-1001234567890, 42, 7], bad: [] });
    expect(parseChatIds("12 abc 0 -0 1.5 @name")).toEqual({ ids: [12], bad: ["abc", "0", "-0", "1.5", "@name"] });
    expect(parseChatIds("")).toEqual({ ids: [], bad: [] });
  });

  it("sends the backup settings back unchanged and no secret it was not given", () => {
    const payload = toPayload(fromView(view), view);
    expect(payload.backup).toEqual({ enabled: true, interval_hours: 12, keep: 5, send_telegram: false, include_usage: true, passphrase: "" });
    expect(payload.telegram.token).toBe("");
    expect(payload.discord.url).toBe("");
    expect(payload.webhook.secret).toBe("");
    expect(payload.telegram.chat_ids).toEqual([-1001, 5]);
    // The shown proxy goes back as shown, so the server keeps its password.
    expect(payload.proxy).toBe("socks5://bot:•••@10.0.0.1:1080");
  });

  it("sends what was typed, trimmed", () => {
    const form = { ...fromView(view), token: " 123:abc ", discordUrl: " https://discord.com/api/webhooks/1/x ", webhookSecret: " s3 ", chatIds: "7, 8" };
    const payload = toPayload(form, view);
    expect(payload.telegram.token).toBe("123:abc");
    expect(payload.discord.url).toBe("https://discord.com/api/webhooks/1/x");
    expect(payload.webhook.secret).toBe("s3");
    expect(payload.telegram.chat_ids).toEqual([7, 8]);
  });

  it("knows when the form differs from what is stored", () => {
    const form = fromView(view);
    expect(isDirty(form, view)).toBe(false);
    expect(isDirty({ ...form, chatIds: "5,\n-1001  " }, view)).toBe(false);
    expect(isDirty({ ...form, proxy: ` ${form.proxy} ` }, view)).toBe(false);
    expect(isDirty({ ...form, language: "fa" }, view)).toBe(true);
    expect(isDirty({ ...form, token: "x" }, view)).toBe(true);
    expect(isDirty({ ...form, events: { ...form.events, data_low_percent: 90 } }, view)).toBe(true);
    expect(isDirty({ ...form, chatIds: "5" }, view)).toBe(true);
  });
});
