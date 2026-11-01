import type { Locale } from './core';

// The host's approval heading (`host.needApprovalFor`, optionally around `verb.switch_mode`) in every locale. A
// persisted card keeps the wording of the locale it was recorded under, and the webview holds only the dictionary it
// shows, so the permission card recognises old headings from this copy. test/i18n.test.ts keeps it equal to the dictionaries
export const APPROVAL_WRAPPERS: Record<Locale, { needApprovalFor: string; switchMode: string }> = {
  'zh-CN': { needApprovalFor: '需要批准：{what}', switchMode: '切换模式' },
  'zh-TW': { needApprovalFor: '需要批准：{what}', switchMode: '切換模式' },
  en: { needApprovalFor: 'Approval needed: {what}', switchMode: 'Switch mode' },
  ja: { needApprovalFor: '承認が必要：{what}', switchMode: 'モード切り替え' },
  ko: { needApprovalFor: '승인 필요: {what}', switchMode: '모드 전환' },
  es: { needApprovalFor: 'Se necesita aprobación: {what}', switchMode: 'Cambio de modo' },
  de: { needApprovalFor: 'Genehmigung erforderlich: {what}', switchMode: 'Modus wechseln' },
  fr: { needApprovalFor: 'Approbation requise : {what}', switchMode: 'Changement de mode' },
  ru: { needApprovalFor: 'Требуется подтверждение: {what}', switchMode: 'Смена режима' },
};
