// Typed wrappers around the Rust commands (src-tauri/src/commands.rs).
// The frontend never talks to anything but the Rust core via IPC.
import { invoke } from '@tauri-apps/api/core';

export type EditionKind = 'free' | 'organization' | 'msp';

export interface EditionInfo {
  kind: EditionKind;
  maxMachines: number | null;
  licensee: string | null;
  pdf: boolean;
  hints: boolean;
  effort: boolean;
  logo: boolean;
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
  license: LicenseView;
}

export interface LicenseView {
  /** `expired`: an MSP license past its last day – the free edition applies until it is renewed. */
  status: 'none' | 'active' | 'expired';
  kind: 'organization' | 'msp' | null;
  licensee: string | null;
  keyId: string | null;
  issued: string | null;
  /** Last day of validity (YYYY-MM-DD), MSP licenses only. */
  expires: string | null;
  /** False only in development builds without a public key. */
  verifiable: boolean;
}

/** Why a key was refused (verified offline in Rust). */
export interface LicenseFailure {
  code: 'malformed' | 'invalidSignature' | 'wrongProduct' | 'unsupportedVersion' | 'expired' | 'unavailable' | 'io';
  date: string | null;
  message: string | null;
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
  overLimit: number;
  machineLimit: number | null;
  errors: ImportErrorView[];
  newerValues: boolean;
  machines: MachineSummary[];
}

export type RiskLevel = 'high' | 'medium' | 'low' | 'info';

/** Rule-of-thumb range in hours. */
export interface Range {
  min: number;
  max: number;
}

export interface KindRow {
  kind: string;
  items: number;
  high: number;
  medium: number;
  low: number;
  notCheckable: number;
  machines: number;
  effort: Range | null;
}

export interface LogSpanView {
  read: number;
  reported: number;
  minDays: number | null;
  maxDays: number | null;
  earliest: string | null;
}

export interface CoverageView {
  full: number;
  limited: number;
  limitations: [string, number][];
  deprecation: LogSpanView;
  sysmon: LogSpanView;
  fileEntries: number;
  fileErrors: number;
  fileSkipped: number;
  notCheckable: [string, number][];
}

export interface SetAsideView {
  file: string;
  hostname: string;
  scannedAt: string;
  why: 'superseded' | 'machineLimit' | string;
}

export interface Overview {
  machines: number;
  items: number;
  high: number;
  medium: number;
  low: number;
  notCheckable: number;
  credentials: number;
  windowsItems: number;
  windowsOccurrences: number;
  effort: Range | null;
  byKind: KindRow[];
  coverage: CoverageView;
  setAside: SetAsideView[];
}

export interface FindingQuery {
  risk?: string | null;
  kind?: string | null;
  origin?: 'own' | 'windows' | 'all';
  search?: string | null;
  offset: number;
  limit: number;
}

export interface FindingRow {
  number: number;
  risk: RiskLevel;
  classification: string;
  status: 'detected' | 'notCheckable' | string;
  reason: string | null;
  rule: string;
  kind: string;
  activation: string;
  origin: 'own' | 'windows';
  machines: number;
  machine: string;
  location: string;
  item: string | null;
  target: string | null;
  effort: Range | null;
}

export interface FindingsPage {
  total: number;
  offset: number;
  rows: FindingRow[];
}

export interface EffortNote {
  sameAs: number | null;
  windows: boolean;
  sizeFactor: number;
  typicalScript: boolean;
  countedOnce: boolean;
}

export interface FindingDetail extends FindingRow {
  reportedActivation: string;
  evidence: { line: number | null; text: string; masked: boolean }[];
  occurrences: {
    machine: string;
    locationKind: string;
    path: string;
    item: string | null;
    target: string | null;
    activation: string;
  }[];
  moreOccurrences: number;
  startedBy: number[];
  starts: number[];
  sameContentAs: number | null;
  fileSize: number | null;
  sha256: string | null;
  details: Record<string, string>;
  /** Translation key of the migration hint; null in the free edition. */
  hint: string | null;
  effortNote: EffortNote | null;
  sources: { publisher: string; title: string; url: string; checked: string }[];
}

export interface ReportSettings {
  customer: string | null;
  logo: { dataUrl: string; width: number; height: number } | null;
}

export const api = {
  appInfo: () => invoke<AppInfo>('app_info'),
  setUiLanguage: (language: string | null) => invoke<LanguageInfo>('set_ui_language', { language }),
  openResultFiles: () => invoke<ImportSummary>('open_result_files'),
  openResultFolder: () => invoke<ImportSummary>('open_result_folder'),
  importPaths: (paths: string[]) => invoke<ImportSummary>('import_paths', { paths }),
  loadedMachines: () => invoke<MachineSummary[]>('loaded_machines'),
  clearResults: () => invoke<void>('clear_results'),
  overview: () => invoke<Overview | null>('overview'),
  findings: (query: FindingQuery) => invoke<FindingsPage>('findings', { query }),
  finding: (number: number) => invoke<FindingDetail | null>('finding', { number }),
  reportSettings: () => invoke<ReportSettings>('report_settings'),
  setReportCustomer: (customer: string | null) => invoke<ReportSettings>('set_report_customer', { customer }),
  chooseReportLogo: () => invoke<ReportSettings>('choose_report_logo'),
  clearReportLogo: () => invoke<ReportSettings>('clear_report_logo'),
  exportExcel: (language: string | null) => invoke<string | null>('export_excel', { language }),
  exportPdf: (language: string | null) => invoke<string | null>('export_pdf', { language }),
  licenseInfo: () => invoke<LicenseView>('license_info'),
  activateLicense: (key: string) => invoke<LicenseView>('activate_license', { key }),
  removeLicense: () => invoke<LicenseView>('remove_license'),
  frontendReady: () => invoke<void>('frontend_ready'),
};
