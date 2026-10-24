// chrome.i18n helpers for the extension pages. Keys are typed against messages.ts.

import type { MsgKey } from "./messages";

export function t(key: MsgKey, ...subs: string[]): string {
  const message = chrome.i18n.getMessage(key, subs);
  if (message === "") {
    throw new Error(`missing UI message "${key}"`);
  }
  return message;
}

/** Fill `data-i18n` (text), `data-i18n-title` and `data-i18n-placeholder` attributes. */
export function localize(root: ParentNode = document): void {
  for (const el of root.querySelectorAll<HTMLElement>("[data-i18n]")) {
    el.textContent = t(el.dataset["i18n"] as MsgKey);
  }
  for (const el of root.querySelectorAll<HTMLElement>("[data-i18n-title]")) {
    el.title = t(el.dataset["i18nTitle"] as MsgKey);
  }
  for (const el of root.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("[data-i18n-placeholder]")) {
    el.placeholder = t(el.dataset["i18nPlaceholder"] as MsgKey);
  }
  document.documentElement.lang = chrome.i18n.getUILanguage();
}
