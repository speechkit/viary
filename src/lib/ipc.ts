// Types and calls shared with the Rust side (src-tauri/src/lib.rs).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";

export type Hotkey = "fn" | "rightOption" | "rightCommand";
export type Language = "auto" | "en" | "zh";
export type Provider = "openAi" | "dashScope";

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
}

export interface EngineInfo {
  id: string;
  name: string;
  kind: string;
  onDevice: boolean;
  live: boolean;
  punctuation: "native" | "model" | "none";
  languageOverride: boolean;
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
  | { kind: "inserted"; label: string; canRaw: boolean }
  | { kind: "copied"; label: string; hint: string }
  | { kind: "failed"; message: string; detail: string; retryable: boolean; alternative: string | null }
  | { kind: "hint"; text: string };

export type PillAction = "undo" | "useRaw" | "retry" | "switchEngine" | "dismiss";

export interface Snapshot {
  settings: Settings;
  engine: EngineStatus;
  keys: { openAi: boolean; dashScope: boolean };
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

export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
  return `${Math.max(1, Math.round(bytes / 1e3))} KB`;
}

export function errorText(error: unknown): string {
  return typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
}
