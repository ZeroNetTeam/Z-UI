/**
 * The user dialog's form: what an admin types (gigabytes, days, Mbit/s),
 * and how it becomes the API's payload (bytes, seconds, bytes per second).
 */
import type { ResetStrategy, User, UserPayload, UserTemplate } from "./api";
import { DAY, bytesToGb, bytesToMbit, gbToBytes, mbitToBytes } from "./users";

export interface UserForm {
  username: string;
  /** What the admin asks for; the panel works out `limited` and `expired`. */
  status: UserPayload["status"];
  dataLimitGb: string;
  /** The expiry itself; the days field moves it. */
  expireAt: number | null;
  /** Days an on-hold user gets once its first connection starts them. */
  onHoldDays: string;
  /** Days after which an on-hold user's time starts even unused (blank: never). */
  onHoldTimeoutDays: string;
  resetStrategy: ResetStrategy;
  ipLimit: string;
  speedUpMbit: string;
  speedDownMbit: string;
  note: string;
  groupIds: number[];
  /** Devices the subscription may be fetched on (blank: any). */
  hwidLimit: string;
  /** A plan to move on to when it runs out of data or time. */
  nextPlan: boolean;
  nextDataGb: string;
  nextDays: string;
  nextCarryOver: boolean;
}

const now = () => Math.floor(Date.now() / 1000);

export function emptyForm(groupIds: number[] = []): UserForm {
  return {
    username: "",
    status: "active",
    dataLimitGb: "",
    expireAt: null,
    onHoldDays: "",
    onHoldTimeoutDays: "",
    resetStrategy: "no_reset",
    ipLimit: "",
    speedUpMbit: "",
    speedDownMbit: "",
    note: "",
    groupIds,
    hwidLimit: "",
    nextPlan: false,
    nextDataGb: "",
    nextDays: "",
    nextCarryOver: false,
  };
}

const days = (seconds: number | null) => (seconds ? String(Math.round(seconds / DAY)) : "");
const count = (value: number) => (value ? String(value) : "");

export function fromUser(user: User): UserForm {
  return {
    username: user.username,
    status: user.status === "disabled" || user.status === "on_hold" ? user.status : "active",
    dataLimitGb: bytesToGb(user.data_limit),
    expireAt: user.expire_at,
    onHoldDays: days(user.on_hold_duration),
    onHoldTimeoutDays: user.on_hold_timeout ? String(Math.max(0, Math.ceil((user.on_hold_timeout - now()) / DAY))) : "",
    resetStrategy: user.reset_strategy,
    ipLimit: count(user.ip_limit),
    speedUpMbit: bytesToMbit(user.speed_up),
    speedDownMbit: bytesToMbit(user.speed_down),
    note: user.note,
    groupIds: user.group_ids,
    hwidLimit: count(user.hwid_limit),
    nextPlan: user.next_plan !== null,
    nextDataGb: bytesToGb(user.next_plan?.data_limit ?? 0),
    nextDays: count(user.next_plan?.expire_days ?? 0),
    nextCarryOver: user.next_plan?.carry_over ?? false,
  };
}

/** The expiry `value` days from now (blank or 0: none). */
export function expiryIn(value: string, from = now()): number | null {
  const n = Number(value);
  return Number.isFinite(n) && n > 0 ? from + Math.round(n * DAY) : null;
}

const whole = (value: string) => {
  const n = Math.floor(Number(value));
  return Number.isFinite(n) && n > 0 ? n : 0;
};

export function toPayload(form: UserForm, at = now()): UserPayload {
  const onHold = form.status === "on_hold";
  const timeout = whole(form.onHoldTimeoutDays);
  return {
    username: form.username.trim(),
    status: form.status,
    data_limit: gbToBytes(form.dataLimitGb),
    expire_at: onHold ? null : form.expireAt,
    on_hold_duration: onHold ? Math.round(Number(form.onHoldDays) * DAY) || null : null,
    on_hold_timeout: onHold && timeout ? at + timeout * DAY : null,
    reset_strategy: form.resetStrategy,
    ip_limit: whole(form.ipLimit),
    speed_up: mbitToBytes(form.speedUpMbit),
    speed_down: mbitToBytes(form.speedDownMbit),
    note: form.note.trim(),
    group_ids: form.groupIds,
    hwid_limit: whole(form.hwidLimit),
    next_plan: form.nextPlan
      ? { data_limit: gbToBytes(form.nextDataGb), expire_days: whole(form.nextDays), carry_over: form.nextCarryOver }
      : null,
  };
}

/** `form` with a template's values; the name and note stay. */
export function withTemplate(form: UserForm, template: UserTemplate, at = now()): UserForm {
  const onHold = template.on_hold && template.expire_days > 0;
  return {
    ...form,
    status: onHold ? "on_hold" : form.status === "on_hold" ? "active" : form.status,
    dataLimitGb: bytesToGb(template.data_limit),
    expireAt: onHold || !template.expire_days ? null : at + template.expire_days * DAY,
    onHoldDays: onHold ? String(template.expire_days) : "",
    resetStrategy: template.reset_strategy,
    ipLimit: count(template.ip_limit),
    hwidLimit: count(template.hwid_limit),
    speedUpMbit: bytesToMbit(template.speed_up),
    speedDownMbit: bytesToMbit(template.speed_down),
    groupIds: template.group_ids.length ? template.group_ids : form.groupIds,
  };
}
