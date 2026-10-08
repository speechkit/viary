// Types and calls shared with the Rust side (src-tauri/src/lib.rs).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import type { Platform } from "./platform";

export type Hotkey = "fn" | "rightOption" | "rightCommand" | "rightAlt" | "ctrlWin" | "shortcut";
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

export type OutputFormat = "srt" | "vtt" | "txt" | "md";

/** How new files are transcribed. */
export interface TranscriptSettings {
  /** An engine id, or null for the dictation engine. */
  engine: string | null;
  language: Language;
  /** Only with speaker separation: "detect", or how many speakers. */
  speakers: "detect" | "two" | "one";
  /** Saved next to the original when a file is done. */
  formats: OutputFormat[];
  /** Start each cue with the speaker's name, when speakers are known. */
  speakerNames: boolean;
}

export interface Settings {
  transcripts: TranscriptSettings;
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
  | { kind: "handsFree"; token: number; startedAt: number; limitMs: number; context: string; live: boolean }
  | { kind: "transcribing"; label: string }
  | { kind: "polishing" }
  | { kind: "inserted"; label: string; canRaw: boolean }
  | { kind: "copied"; label: string; hint: string }
  | { kind: "failed"; message: string; detail: string; retryable: boolean; alternative: string | null }
  | { kind: "hint"; text: string };

export type PillAction = "undo" | "useRaw" | "retry" | "switchEngine" | "dismiss" | "skipPolish" | "stop";

/**
 * What recognition can tell beyond passage text and times. speechkit 0.5
 * provides none of these; dev builds can mock them (VIARY_MOCK_SPEECHKIT=1).
 */
export interface SpeechCaps {
  speakers: boolean;
  wordTimings: boolean;
  wordConfidence: boolean;
  /** The capabilities come from the dev fixture, not from speechkit. */
  mock: boolean;
}

export interface Desktop {
  session: "wayland" | "x11";
  /** `hold` reports down and up; `toggle` (GNOME 46) only presses. */
  shortcut: { mode: "hold" | "toggle" | "unknown"; bound: boolean };
  typing: "portal" | "clipboard";
  /** GNOME already allowed typing through the RemoteDesktop portal. */
  portalAllowed: boolean;
}

export interface Snapshot {
  platform: Platform;
  settings: Settings;
  speechCaps: SpeechCaps;
  engine: EngineStatus;
  keys: { openAi: boolean; dashScope: boolean; customPolish: boolean };
  /** Windows reports the microphone switch; macOS asks when recording starts. */
  permissions: { accessibility: boolean; inputMonitoring: boolean; microphone?: boolean };
  hotkeyActive: boolean;
  hotkeyName: string;
  pill: PillView;
  /** Dictation paused from the tray, until a time (ms since the epoch) or until resumed. */
  paused: { until: number | null } | null;
  /** Viary starts when the user signs in. */
  autostart: boolean;
  /** The GNOME session, talk shortcut, and typing method on Linux; null elsewhere. */
  desktop: Desktop | null;
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

/** A word with its own timing, when the engine reports one. */
export interface Word {
  text: string;
  startMs: number;
  endMs: number;
  /** 0 to 1, when the engine reports it. */
  confidence: number | null;
}

/** One recognized passage: a segment of speech, with its own timing. */
export interface Passage {
  startMs: number;
  endMs: number;
  text: string;
  /** The recognized text, once the passage has been edited. */
  original?: string | null;
  /** Index into the note's speakers, when speakers were found. */
  speaker: number | null;
  /** Only when the engine timed each word. */
  words: Word[] | null;
}

export interface ActionItem {
  who: string | null;
  text: string;
  atMs: number | null;
  done: boolean;
}

export interface Note {
  id: string;
  title: string;
  createdAt: number;
  durationMs: number;
  engine: { name: string; kind: string; onDevice: boolean };
  passages: Passage[];
  /** Empty unless speakers were found. */
  speakers: { name: string }[];
  marks: number[];
  summary: string | null;
  actions: ActionItem[];
  summaryError: string | null;
  /** Loudness for the player's waveform, 0 to 1. */
  peaks: number[];
  audioPath: string | null;
  /** Parts of the recording not yet joined into its audio file. */
  pendingAudio?: { file: string; rate: number }[];
}

export type StepState = "done" | "now" | "todo";

/** The note being recorded or finished, if any. */
export interface RecorderState {
  phase: "idle" | "recording" | "paused" | "finishing";
  id: string | null;
  title: string;
  /** Recorded time before `runningSince`. */
  elapsedMs: number;
  /** When the running part began (epoch ms); null while paused. */
  runningSince: number | null;
  marks: number[];
  steps: { label: string; state: StepState }[];
  error: string | null;
  /** When `error` happened (epoch ms). */
  errorAt: number | null;
  /** The note just saved, to open. */
  saved: string | null;
  /** Recording while the voice engine finishes loading; pause and stop wait. */
  waitingForEngine: boolean;
}

export interface LiveLine {
  startMs: number;
  text: string;
}

export type NoteFormat = "markdown" | "text";

export type JobState =
  | { kind: "waiting" }
  | { kind: "running"; stage: string; progress: number | null; secondsLeft: number | null }
  | { kind: "failed"; error: string };

/** A file in the transcription queue. */
export interface Job {
  id: string;
  path: string;
  name: string;
  durationMs: number | null;
  state: JobState;
}

/** A transcript in the library, without its passages. */
export interface TranscriptSummary {
  id: string;
  name: string;
  source: string;
  sourceExists: boolean;
  createdAt: number;
  durationMs: number;
  engine: { name: string; kind: string; onDevice: boolean };
  speakers: { name: string }[];
  saved: string[];
}

export interface TranscriptDoc extends TranscriptSummary {
  passages: Passage[];
}

export interface SearchHit {
  id: string;
  name: string;
  count: number;
  /** The first match. */
  atMs: number;
  before: string;
  match: string;
  after: string;
}

export interface Cue {
  n: number;
  startMs: number;
  endMs: number;
  text: string;
  /** Speaker index, for highlighting and overlaps. */
  speaker: number | null;
  warning: string | null;
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
  /** Opens Voice Notes and starts a recording, as ⌥⌘N does. */
  newVoiceNote: () => invoke<void>("new_voice_note"),
  notes: () => invoke<Note[]>("notes_list"),
  recorder: () => invoke<RecorderState>("note_recorder_state"),
  /** The live transcript of the note being recorded, if any. */
  noteLive: () => invoke<{ id: string; lines: LiveLine[]; partial: string; partialStartMs: number | null } | null>("note_live"),
  notePause: () => invoke<void>("note_pause"),
  noteResume: () => invoke<void>("note_resume"),
  noteMark: () => invoke<void>("note_mark"),
  noteDiscard: () => invoke<void>("note_discard"),
  noteStop: () => invoke<void>("note_stop"),
  /** Renames a note, or the one being recorded. */
  noteRename: (id: string, title: string) => invoke<void>("note_rename", { id, title }),
  noteSetAction: (id: string, index: number, done: boolean) => invoke<void>("note_set_action", { id, index, done }),
  noteSetMarks: (id: string, marks: number[]) => invoke<void>("note_set_marks", { id, marks }),
  noteDelete: (id: string) => invoke<void>("note_delete", { id }),
  noteSummarize: (id: string) => invoke<void>("note_summarize", { id }),
  noteText: (id: string, format: NoteFormat) => invoke<string>("note_text", { id, format }),
  noteExport: (id: string, format: NoteFormat, path: string) => invoke<void>("note_export", { id, format, path }),
  /** Opens Transcripts and adds these files and folders to the queue. */
  transcribeFiles: (paths: string[]) => invoke<void>("transcribe_files", { paths }),
  jobs: () => invoke<Job[]>("transcript_jobs"),
  retryJob: (id: string) => invoke<void>("transcript_job_retry", { id }),
  cancelJob: (id: string) => invoke<void>("transcript_job_cancel", { id }),
  transcripts: () => invoke<TranscriptSummary[]>("transcripts_list"),
  transcript: (id: string) => invoke<TranscriptDoc>("transcript_get", { id }),
  searchTranscripts: (query: string) => invoke<SearchHit[]>("transcripts_search", { query }),
  editPassage: (id: string, index: number, text: string) => invoke<void>("transcript_edit_passage", { id, index, text }),
  renameSpeaker: (id: string, index: number, name: string) => invoke<void>("transcript_rename_speaker", { id, index, name }),
  /** Replaces every match of `find`, ignoring case; returns how many. */
  replaceAll: (id: string, find: string, replace: string) => invoke<number>("transcript_replace_all", { id, find, replace }),
  deleteTranscript: (id: string) => invoke<void>("transcript_delete", { id }),
  cues: (id: string, speakerNames: boolean) => invoke<Cue[]>("transcript_cues", { id, speakerNames }),
  exportTranscript: (id: string, format: OutputFormat, path: string) => invoke<void>("transcript_export", { id, format, path }),
  updateTranscriptSettings: (patch: Partial<TranscriptSettings>) => invoke<void>("update_transcript_settings", { patch }),
  /** File extensions speechkit decodes in this build, for the file dialog. */
  audioExtensions: () => invoke<string[]>("audio_extensions"),
  hidePopover: () => invoke<void>("hide_popover"),
  /** Pauses dictation for `minutes`, or until resumed. */
  pauseDictation: (minutes: number | null) => invoke<void>("pause_dictation", { minutes }),
  resumeDictation: () => invoke<void>("resume_dictation"),
  setAutostart: (on: boolean) => invoke<void>("set_autostart", { on }),
  /** While on, the talk key only reports itself (`hotkey` events) and starts no dictation. */
  setKeyTest: (on: boolean) => invoke<void>("set_key_test", { on }),
  /** Closes the setup window for good; Viary goes on in the tray. */
  finishSetup: () => invoke<void>("finish_setup"),
  /** Asks GNOME for the talk shortcut (Linux). */
  bindShortcut: () => invoke<void>("bind_shortcut"),
  /** How Viary types on Wayland; `portal` asks GNOME now (Linux). */
  setTyping: (method: "portal" | "clipboard") => invoke<void>("set_typing", { method }),
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

/** Calls `onKey` as the talk key goes down and up. */
export function useHotkey(onKey: (down: boolean) => void) {
  useEffect(() => {
    const unlisten = listen<"down" | "up">("hotkey", (e) => onKey(e.payload === "down"));
    return () => {
      unlisten.then((stop) => stop());
    };
  }, [onKey]);
}
export const useHistory = () => useRefreshing(api.history);

/** Calls `fetch` now and on `event` (and window focus, with `onFocus`),
 *  not on every `state-changed`: for lists that change on their own. */
function useReloadedOn<T>(fetch: () => Promise<T>, event: string, onFocus = false): T | null {
  const [value, setValue] = useState<T | null>(null);
  useEffect(() => {
    const reload = () => {
      fetch().then(setValue, (error) => console.error(error));
    };
    reload();
    const stop = listen(event, reload);
    if (onFocus) window.addEventListener("focus", reload);
    return () => {
      stop.then((f) => f());
      window.removeEventListener("focus", reload);
    };
  }, [fetch, event, onFocus]);
  return value;
}

/** The notes, newest first, reloaded when the backend changes them. */
export const useNotes = (): Note[] | null => useReloadedOn(api.notes, "notes-changed");

/** The transcription queue, kept current by its events. */
export function useJobs(): Job[] {
  const [jobs, setJobs] = useState<Job[]>([]);
  useEffect(() => {
    api.jobs().then(setJobs, console.error);
    const stop = listen<Job[]>("transcript-jobs", ({ payload }) => setJobs(payload));
    return () => {
      stop.then((f) => f());
    };
  }, []);
  return jobs;
}

/** The library, newest first, reloaded when it changes, and on focus, since
 *  an original may have moved meanwhile. */
export const useTranscripts = (): TranscriptSummary[] | null => useReloadedOn(api.transcripts, "transcripts-changed", true);

/** The recorder's state, kept current by its events. */
export function useRecorder(): RecorderState | null {
  const [state, setState] = useState<RecorderState | null>(null);
  useEffect(() => {
    // An error from before the page opened is old news, unless it is what
    // opened the page (⌥⌘N with no voice engine fails as the page opens).
    api.recorder().then((initial) => {
      const fresh = initial.errorAt !== null && Date.now() - initial.errorAt < 5_000;
      setState(fresh ? initial : { ...initial, error: null });
    }, console.error);
    const stop = listen<RecorderState>("note-recorder", ({ payload }) => setState(payload));
    return () => {
      stop.then((f) => f());
    };
  }, []);
  return state;
}

/** "4:21", or "1:02:05" past an hour. */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

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

/** The microphone Viary records from: the chosen one, or the system default's name. */
export function useMicrophoneName(s: Snapshot | null): string {
  const [fallback, setFallback] = useState("Default microphone");
  const chosen = s?.settings.microphone ?? null;
  useEffect(() => {
    if (chosen) return;
    api.microphones().then(
      (list) => setFallback(list.find((m) => m.isDefault)?.name ?? "Default microphone"),
      () => {},
    );
  }, [chosen]);
  return chosen ?? fallback;
}

export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
  return `${Math.max(1, Math.round(bytes / 1e3))} KB`;
}

export function errorText(error: unknown): string {
  return typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
}
