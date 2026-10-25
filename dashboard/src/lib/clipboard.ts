import i18n from "@/i18n";
import { toast } from "@/lib/toast";

/** Copy the old way: a text box selected and `copy` run on it. Browsers give no `navigator.clipboard` to a page on plain HTTP, which is how most panels are first opened. */
function copyByHand(text: string): boolean {
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.style.position = "fixed";
  area.style.opacity = "0";
  area.style.pointerEvents = "none";
  document.body.appendChild(area);
  area.select();
  area.setSelectionRange(0, text.length);
  let done = false;
  try {
    done = document.execCommand("copy");
  } catch {
    done = false;
  }
  document.body.removeChild(area);
  return done;
}

/** Put `text` on the clipboard; whether it got there. */
export async function copyText(text: string): Promise<boolean> {
  if (typeof navigator !== "undefined" && navigator.clipboard && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Refused (no focus, no permission): the old way may still work.
    }
  }
  return copyByHand(text);
}

/** Copy, and say how it went: "Copied", or that it could not be, with a way to try again. */
export async function copyAndTell(text: string, what?: string): Promise<boolean> {
  const done = await copyText(text);
  if (done) {
    toast(what ? i18n.t("toast.copiedWhat", { what }) : i18n.t("toast.copied"), "success");
  } else {
    toast(i18n.t("toast.copyFailed"), "error", {
      label: i18n.t("toast.tryAgain"),
      run: () => void copyAndTell(text, what),
    });
  }
  return done;
}
