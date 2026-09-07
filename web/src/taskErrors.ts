import type { TFunc } from "./i18n";

/**
 * Server task/op error text is authored in English and passes through the SPA
 * untranslated (issue #117). Known high-frequency patterns get a translated
 * rendering; unknown text is shown verbatim with a "server text" label —
 * never silently dropped, so forensics keep the exact server string.
 */
export function taskErrorText(t: TFunc, raw: string): { text: string; translated: boolean } {
  // 不可回放(log_reader.rs):base 处 refs 无 checkpoint 锚定——用户在
  // 中文界面实测撞到的报错(issue #117 起源)。
  if (raw.includes("are not replayable")) {
    return { text: t("taskerr.replayable"), translated: true };
  }
  return { text: `${raw}（${t("taskerr.original")}）`, translated: false };
}
