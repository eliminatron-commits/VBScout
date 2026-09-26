// Typed wrappers around the Rust commands (src-tauri/src/commands.rs).
// The frontend never talks to anything but the Rust core via IPC.
import { invoke } from '@tauri-apps/api/core';

export type EditionKind = 'free' | 'organization' | 'msp';

export interface EditionInfo {
  kind: EditionKind;
  maxMachines: number | null;
}

export interface LanguageInfo {
  tag: string;
  reviewed: boolean;
}

export interface AppInfo {
  productName: string;
  version: string;
  platform: string;
  edition: EditionInfo;
  language: LanguageInfo;
  uiLanguageSetting: string | null;
  translationsUrl: string;
  rulesAsOf: string;
}

export interface MachineSummary {
  scanId: string;
  fileName: string;
  path: string;
  hostname: string;
  domain: string | null;
  os: string | null;
  scannedAt: string;
  coverage: 'full' | 'limited' | string;
  limitations: string[];
  findings: number;
  notCheckable: number;
  collectorVersion: string;
}

export interface ImportErrorView {
  file: string;
  code: 'notResultFile' | 'wrongFormat' | 'newerSchema' | 'invalid' | 'tooLarge' | 'io';
  message: string | null;
  found: number | null;
  supported: number | null;
}

export interface ImportSummary {
  cancelled: boolean;
  loaded: number;
  duplicates: number;
  errors: ImportErrorView[];
  newerValues: boolean;
  machines: MachineSummary[];
}

export const api = {
  appInfo: () => invoke<AppInfo>('app_info'),
  setUiLanguage: (language: string | null) => invoke<LanguageInfo>('set_ui_language', { language }),
  openResultFiles: () => invoke<ImportSummary>('open_result_files'),
  openResultFolder: () => invoke<ImportSummary>('open_result_folder'),
  importPaths: (paths: string[]) => invoke<ImportSummary>('import_paths', { paths }),
  loadedMachines: () => invoke<MachineSummary[]>('loaded_machines'),
  clearResults: () => invoke<void>('clear_results'),
  frontendReady: () => invoke<void>('frontend_ready'),
};
