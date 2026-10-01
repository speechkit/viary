// Types and calls shared with the Rust side (src-tauri/src/lib.rs).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";

export type Hotkey = "fn" | "rightOption" | "rightCommand";
export type Language = "auto" | "en" | "zh";
export type Provider = "openAi" | "dashScope" | "customPolish";

export interface LocalModel {
  id: string;
  name: string;
  path: string;
  family: string;
  sizeBytes: number;
}

export interface OpenAiSettings {
  baseUrl: string;
  model: string;
  mode: "file" | "realtime";
}

export interface DashScopeSettings {
  model: string;
  region: "china" | "international";
}

/** A word the engine should get right: a name, a term, or a fix for a mishearing. */
export interface DictionaryEntry {
  /** Empty for a word not saved yet. */
  id: string;
  /** How to write it. */
  word: string;
  /** What the engine hears instead, replaced after recognition. */
  soundsLike: string[];
  kind: "term" | "person";
  boost: "normal" | "strong";
  /** App names it applies in; empty for every app. */
  apps: string[];
}

/** The most dictionary words: speechkit's limit on hotwords. */
export const MAX_WORDS = 256;

export type Tone = "formal" | "casual" | "asSpoken" | "literal";
export type PolishProvider = "local" | "custom" | "openAi" | "dashScope";

export interface PolishSettings {
  enabled: boolean;
  removeFillers: boolean;
  selfCorrections: boolean;
  formatLists: boolean;
  appTone: boolean;
  translate: boolean;
  /** The language to translate into, by its English name. */
  translateTo: string;
  /** Where the language model runs: an OpenAI-compatible server. */
  provider: PolishProvider;
  /** The base URL of a local or custom OpenAI-compatible server. */
  baseUrl: string;
  /** Empty until the user names one: Viary never picks a model. */
  model: string;
  tones: { app: string; tone: Tone }[];
  /** The tone in apps without their own. */
  defaultTone: Tone;
}

export interface Settings {
  localModels: LocalModel[];
  activeEngine: string | null;
  vadModel: string | null;
  punctModel: string | null;
  provider: string;
  threads: number;
  microphone: string | null;
  language: Language;
  hotkey: Hotkey;
  keepRecordingsDays: number;
  openai: OpenAiSettings;
  dashscope: DashScopeSettings;
  dictionary: DictionaryEntry[];
  polish: PolishSettings;
}

export interface EngineInfo {
  id: string;
  name: string;
  kind: string;
  onDevice: boolean;
  live: boolean;
  punctuation: "native" | "model" | "none";
  languageOverride: boolean;
  /** How it uses the dictionary. */
  dictionary: "hotwords" | "prompt" | "replacements";
}

export interface EngineStatus {
  active: EngineInfo | null;
  loading: string | null;
  failed: string | null;
  error: string | null;
}

export interface FamilyInfo {
  id: string;
  label: string;
  description: string;
  tags: string[];
  streaming: boolean;
  needsVad: boolean;
  nativePunctuation: boolean;
}

export interface ModelInspection {
  path: string;
  name: string;
  layout: string;
  families: FamilyInfo[];
  sizeBytes: number;
  vadNearby: string | null;
}

export type PillView =
  | { kind: "idle" }
  | { kind: "listening"; token: number; startedAt: number; context: string; live: boolean }
  | { kind: "transcribing"; label: string }
  | { kind: "polishing" }
  | { kind: "inserted"; label: string; canRaw: boolean }
  | { kind: "copied"; label: string; hint: string }
  | { kind: "failed"; message: string; detail: string; retryable: boolean; alternative: string | null }
  | { kind: "hint"; text: string };

export type PillAction = "undo" | "useRaw" | "retry" | "switchEngine" | "dismiss" | "skipPolish";

export interface Snapshot {
  settings: Settings;
  engine: EngineStatus;
  keys: { openAi: boolean; dashScope: boolean; customPolish: boolean };
  permissions: { accessibility: boolean; inputMonitoring: boolean };
  hotkeyActive: boolean;
  hotkeyName: string;
  pill: PillView;
  families: [string, FamilyInfo][];
  punctLayout: string | null;
}

export type HistoryStatus = "inserted" | "copied" | "undone" | "failed" | "transcribed";

export interface HistoryItem {
  id: string;
  createdAt: number;
  app: string;
  durationMs: number;
  text: string;
  raw: string;
  punctuated: string | null;
  engine: { name: string; kind: string; onDevice: boolean };
  status: HistoryStatus;
  error: string | null;
  recording: string | null;
  recordingPath: string | null;
  words: number;
}

export interface Preferences {
  microphone?: string | null;
  language?: Language;
  hotkey?: Hotkey;
  keepRecordingsDays?: number;
  provider?: string;
  threads?: number;
}

/** A model folder's name without the `sherpa-onnx-` every archive starts with. */
export function shortName(name: string): string {
  return name.replace(/^sherpa-onnx-/, "");
}

async function state(): Promise<Snapshot> {
  const s = await invoke<Snapshot>("get_state");
  s.settings.localModels.forEach((m) => (m.name = shortName(m.name)));
  if (s.engine.active) s.engine.active.name = shortName(s.engine.active.name);
  return s;
}

async function history(): Promise<HistoryItem[]> {
  const items = await invoke<HistoryItem[]>("history_list");
  items.forEach((h) => (h.engine.name = shortName(h.engine.name)));
  return items;
}

export const api = {
  state,
  microphones: () => invoke<{ name: string; isDefault: boolean }[]>("list_microphones"),
  inspectModel: (path: string) => invoke<ModelInspection>("inspect_model", { path }),
  addLocalModel: (path: string, family: string) => invoke<string>("add_local_model", { path, family }),
  removeLocalModel: (id: string) => invoke<void>("remove_local_model", { id }),
  setActiveEngine: (id: string) => invoke<void>("set_active_engine", { id }),
  setVadModel: (path: string | null) => invoke<void>("set_vad_model", { path }),
  setPunctModel: (path: string | null) => invoke<void>("set_punct_model", { path }),
  setOpenAi: (openai: OpenAiSettings) => invoke<void>("set_openai", { openai }),
  setDashScope: (dashscope: DashScopeSettings) => invoke<void>("set_dashscope", { dashscope }),
  saveApiKey: (provider: Provider, key: string) => invoke<void>("save_api_key", { provider, key }),
  deleteApiKey: (provider: Provider) => invoke<void>("delete_api_key", { provider }),
  setPreferences: (prefs: Preferences) => invoke<void>("set_preferences", { prefs }),
  saveWord: (entry: DictionaryEntry) => invoke<string>("dictionary_save", { entry }),
  removeWord: (id: string) => invoke<void>("dictionary_remove", { id }),
  /** Changes only the fields given, so quick changes never undo each other. */
  updatePolish: (patch: Partial<Omit<PolishSettings, "tones">>) => invoke<void>("update_polish", { patch }),
  /** Sets an app's polish tone, or takes the app off the list with `null`. */
  setAppTone: (app: string, tone: Tone | null) => invoke<void>("set_app_tone", { target: app, tone }),
  testPolishConnection: (patch: Pick<PolishSettings, "provider" | "baseUrl" | "model">, key: string | null) =>
    invoke<void>("test_polish_connection", { patch, key }),
  polishPreview: (app: string, text: string) => invoke<string>("polish_preview", { target: app, text }),
  history,
  deleteHistory: (id: string) => invoke<void>("history_delete", { id }),
  retranscribe: (id: string) => invoke<void>("history_retranscribe", { id }),
  copy: (text: string) => invoke<void>("copy_text", { text }),
  pill: (action: PillAction) => invoke<void>("pill_action", { action }),
  fitPill: (width: number, height: number) => invoke<void>("fit_pill", { width, height }),
  requestPermission: (kind: "accessibility" | "inputMonitoring" | "microphone") =>
    invoke<void>("request_permission", { kind }),
  openMain: (page: string) => invoke<void>("open_main", { page }),
  hidePopover: () => invoke<void>("hide_popover"),
  quit: () => invoke<void>("quit"),
};

/** Calls `fetch` now and whenever the backend says its state changed. */
function useRefreshing<T>(fetch: () => Promise<T>): [T | null, () => void] {
  const [value, setValue] = useState<T | null>(null);
  const reload = useCallback(() => {
    fetch().then(setValue, (error) => console.error(error));
  }, [fetch]);
  useEffect(() => {
    reload();
    const unlisten = listen("state-changed", reload);
    window.addEventListener("focus", reload);
    return () => {
      unlisten.then((stop) => stop());
      window.removeEventListener("focus", reload);
    };
  }, [reload]);
  return [value, reload];
}

export const useSnapshot = () => useRefreshing(api.state);
export const useHistory = () => useRefreshing(api.history);

/** The engine's name for the menu bar and the sidebar. */
export function engineLabel(info: EngineInfo): string {
  return info.onDevice ? `On-device · ${info.name}` : `Cloud · ${info.kind} · ${info.name}`;
}

export function engineName(snapshot: Snapshot, id: string): string {
  if (id === "openai") return "OpenAI";
  if (id === "dashscope") return "DashScope";
  const local = snapshot.settings.localModels.find((m) => `local:${m.id}` === id);
  return local?.name ?? "engine";
}

/** App names seen in History, most used first. */
export function knownApps(history: HistoryItem[] | null): string[] {
  const counts = new Map<string, number>();
  for (const h of history ?? []) if (h.app) counts.set(h.app, (counts.get(h.app) ?? 0) + 1);
  return [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([app]) => app);
}

/** The polish tone that applies in `app`, as the backend decides it. */
export function toneIn(polish: PolishSettings, app: string): Tone {
  const own = polish.tones.find((t) => t.app.toLowerCase() === app.toLowerCase())?.tone ?? polish.defaultTone;
  if (own === "literal") return "literal";
  return polish.appTone ? own : "asSpoken";
}

export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
  return `${Math.max(1, Math.round(bytes / 1e3))} KB`;
}

export function errorText(error: unknown): string {
  return typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
}
