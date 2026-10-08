// Dev only: renders Viary's windows in a browser against a mocked backend,
// to compare them with the design. Open /preview.html?w=main|popover|pill|setup.

import "@fontsource/geist-sans/400.css";
import "@fontsource/geist-sans/500.css";
import "@fontsource/geist-sans/600.css";
import "@fontsource/geist-mono/400.css";
import "@fontsource/newsreader/400.css";
import "@fontsource/newsreader/500.css";
import "./styles.css";

import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { createRoot } from "react-dom/client";
import fixture from "../src-tauri/fixtures/voice-note.json";
import type {
  Cue,
  DictionaryEntry,
  HistoryItem,
  Job,
  Note,
  Passage,
  PillView,
  PolishSettings,
  Provider,
  RecorderState,
  Snapshot,
  Tone,
  TranscriptDoc,
  TranscriptSettings,
} from "./lib/ipc";
import { PLATFORM } from "./lib/platform";

const params = new URLSearchParams(location.search);
const label = params.get("w") ?? "main";
mockWindows(label);
document.documentElement.dataset.window = label;

const now = Date.now();
// ?caps=mock shows what speechkit cannot do yet (speakers, word timing);
// without it, the preview has speechkit 0.5's real capabilities.
const mockCaps = params.get("caps") === "mock";

// ?os=windows or ?os=linux renders that system's windows and wording.
const HOTKEYS = {
  macos: { hotkey: "fn", name: "fn" },
  windows: { hotkey: "rightAlt", name: "Right Alt" },
  linux: { hotkey: "shortcut", name: "Ctrl+Alt+Space" },
} as const;

const KEY_NAMES = { fn: "fn", rightOption: "right ⌥", rightCommand: "right ⌘", rightAlt: "Right Alt", ctrlWin: "Ctrl + Win", shortcut: "Ctrl+Alt+Space" };

const snapshot: Snapshot = {
  platform: PLATFORM,
  speechCaps: { speakers: mockCaps, wordTimings: mockCaps, wordConfidence: mockCaps, mock: mockCaps },
  settings: {
    // ?fileEngine=dashscope (no key here) or local:gone shows an engine for files that can't be used.
    transcripts: { engine: params.get("fileEngine"), language: "auto", speakers: "detect", formats: ["srt", "vtt"], speakerNames: true },
    localModels: [
      { id: "1", name: "sherpa-onnx-streaming-zipformer-en-2023-06-26", path: "/m/z", family: "streaming-transducer", sizeBytes: 70e6 },
      { id: "2", name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17", path: "/m/s", family: "sense-voice", sizeBytes: 240e6 },
    ],
    activeEngine: "local:2",
    vadModel: "/Users/me/.cache/speechkit/models/silero_vad.onnx",
    punctModel: null,
    provider: "cpu",
    threads: 2,
    microphone: null,
    language: "auto",
    hotkey: HOTKEYS[PLATFORM].hotkey,
    keepRecordingsDays: 7,
    openai: { baseUrl: "https://api.openai.com/v1", model: "", mode: "file" },
    dashscope: { model: "", region: "china" },
    dictionary: [
      { id: "w1", word: "speechkit", soundsLike: [], kind: "term", boost: "strong", apps: [] },
      { id: "w2", word: "Mei Lin", soundsLike: ["may lin"], kind: "person", boost: "normal", apps: [] },
      { id: "w3", word: "sherpa-onnx", soundsLike: ["sherpa onyx"], kind: "term", boost: "strong", apps: ["VS Code", "Slack"] },
      { id: "w4", word: "cargo clippy", soundsLike: ["cargo clippie"], kind: "term", boost: "normal", apps: ["Terminal"] },
      { id: "w5", word: "通义千问", soundsLike: ["tong yi qian wen"], kind: "term", boost: "strong", apps: [] },
    ],
    polish: {
      enabled: true,
      removeFillers: true,
      selfCorrections: true,
      formatLists: true,
      appTone: true,
      translate: false,
      translateTo: "English",
      provider: "local",
      baseUrl: "http://localhost:11434/v1",
      model: "qwen3:8b",
      tones: [
        { app: "Mail", tone: "formal" },
        { app: "Slack", tone: "casual" },
        { app: "Notes", tone: "asSpoken" },
        { app: "VS Code", tone: "literal" },
        { app: "Terminal", tone: "literal" },
      ],
      defaultTone: "asSpoken",
    },
  },
  engine: {
    active: { id: "local:2", name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17", kind: "SenseVoice", onDevice: true, live: false, punctuation: "native", languageOverride: true, dictionary: "replacements" },
    loading: null,
    failed: null,
    error: null,
  },
  keys: { openAi: true, dashScope: false, customPolish: false },
  permissions: { accessibility: true, inputMonitoring: true },
  hotkeyActive: true,
  hotkeyName: HOTKEYS[PLATFORM].name,
  pill: { kind: "idle" },
  paused: null,
  autostart: true,
  desktop:
    PLATFORM === "linux"
      ? {
          session: "wayland",
          shortcut: { mode: params.get("gnome") === "46" ? "toggle" : "hold", bound: false },
          typing: "clipboard",
          portalAllowed: false,
          // ?ext=installed or ?ext=active: the extension's other states.
          extension: (params.get("ext") as "installed" | "active" | null) ?? "missing",
        }
      : null,
  families: [
    ["1", { id: "streaming-transducer", label: "Streaming Zipformer", description: "Shows words while you speak. Commits a phrase at each pause.", tags: ["Live preview", "Hotwords"], streaming: true, needsVad: false, nativePunctuation: false }],
    ["2", { id: "sense-voice", label: "SenseVoice", description: "Fast and punctuated. Transcribes each phrase after a pause.", tags: ["5 languages", "Punctuation"], streaming: false, needsVad: true, nativePunctuation: true }],
  ],
  punctLayout: null,
};

const history: HistoryItem[] = [
  { id: "a", createdAt: now - 60_000, app: "Mail", durationMs: 26_000, text: "Hi Mei, thanks for the notes on the release plan. We tag 0.2.0 on Thursday, once the nightly run is green.", raw: "hi mei thanks for the notes on the release plan we tag 0.2.0 on thursday once the nightly run is green", punctuated: "Hi Mei, thanks for the notes on the release plan. We tag 0.2.0 on Thursday, once the nightly run is green.", engine: { name: "sherpa-onnx-streaming-zipformer-en-2023-06-26", kind: "Streaming Zipformer", onDevice: true }, status: "inserted", error: null, recording: null, recordingPath: null, words: 21 },
  { id: "b", createdAt: now - 3_600_000, app: "Slack", durationMs: 9_000, text: "", raw: "", punctuated: null, engine: { name: "gpt-4o-transcribe", kind: "OpenAI", onDevice: false }, status: "failed", error: "backend `openai-http` failed: connection reset", recording: null, recordingPath: null, words: 0 },
];

// --- Voice Notes ----------------------------------------------------------

/** A silent WAV of `ms`, so the preview's player can play and seek. */
function silence(ms: number): string {
  const rate = 8000;
  const n = Math.round((rate * ms) / 1000);
  const buf = new ArrayBuffer(44 + n);
  const v = new DataView(buf);
  const str = (o: number, t: string) => [...t].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
  str(0, "RIFF");
  v.setUint32(4, 36 + n, true);
  str(8, "WAVEfmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, rate, true);
  v.setUint32(28, rate, true);
  v.setUint16(32, 1, true);
  v.setUint16(34, 8, true);
  str(36, "data");
  v.setUint32(40, n, true);
  new Uint8Array(buf, 44).fill(128);
  return URL.createObjectURL(new Blob([buf], { type: "audio/wav" }));
}

/** The fixture's passages that ended by `ms`, with speakers and words only under ?caps=mock. */
function fixturePassages(ms: number): Passage[] {
  return fixture.passages
    .map((p) => {
      const words = p.words.map(([text, startMs, endMs, confidence]) => ({
        text: text as string,
        startMs: startMs as number,
        endMs: endMs as number,
        confidence: mockCaps ? (confidence as number) : null,
      }));
      return {
        startMs: words[0].startMs,
        endMs: words[words.length - 1].endMs,
        text: words.map((w) => w.text).join(" "),
        speaker: mockCaps ? p.speaker : null,
        words: mockCaps ? words : null,
      };
    })
    .filter((p) => p.endMs <= ms);
}

function makeNote(id: string, title: string, createdAt: number, durationMs: number, marks: number[]): Note {
  const passages = fixturePassages(durationMs);
  const who = (i: number) => (mockCaps ? fixture.speakers[i].split(" ")[0] : null);
  return {
    id,
    title,
    createdAt,
    durationMs,
    engine: { name: "sense-voice-zh-en-ja-ko-yue-int8-2024-07-17", kind: "SenseVoice", onDevice: true },
    passages,
    speakers: mockCaps ? [...new Set(passages.map((p) => p.speaker!))].sort().map((i) => ({ name: fixture.speakers[i] })) : [],
    marks,
    summary: passages.length > 1 ? "The beta ships Thursday once the nightly run is green. The demo is Friday afternoon." : null,
    actions:
      passages.length > 2
        ? [
            { who: who(1), text: "Write the release notes and changelog", atMs: 9300, done: false },
            { who: who(1), text: "Book the big room for the demo", atMs: 19200, done: true },
          ].slice(0, passages.length > 4 ? 2 : 1)
        : [],
    summaryError: null,
    peaks: Array.from({ length: 110 }, (_, i) => {
      const at = (i / 110) * durationMs;
      return passages.some((p) => at >= p.startMs && at < p.endMs) ? 0.25 + Math.abs(Math.sin(i * 0.61) * Math.cos(i * 0.19)) * 0.75 : 0.02;
    }),
    audioPath: silence(durationMs),
  };
}

const notes: Note[] =
  params.get("notes") === "0"
    ? []
    : [
        makeNote("n1", "Launch plan sync", now - 40 * 60_000, 23_000, [6_200]),
        makeNote("n2", "Idea: tray drop target", now - 3 * 3_600_000, 9_000, []),
        makeNote("n3", "Standup notes", now - 2 * 86_400_000, 18_000, []),
      ];

const idle: RecorderState = { phase: "idle", id: null, title: "", elapsedMs: 0, runningSince: null, marks: [], steps: [], error: null, errorAt: null, saved: null, waitingForEngine: false };
let recorder: RecorderState = { ...idle };
let recTimer: ReturnType<typeof setInterval> | undefined;

function setRecorder(next: Partial<RecorderState>) {
  recorder = { ...recorder, ...next };
  emit("note-recorder", recorder);
}

const recElapsed = () => recorder.elapsedMs + (recorder.runningSince ? Date.now() - recorder.runningSince : 0);

let livePayload: unknown = null;

/** Plays the fixture as live text while "recording". */
function runRecorder() {
  clearInterval(recTimer);
  recTimer = setInterval(() => {
    if (recorder.phase !== "recording") return;
    const ms = recElapsed();
    const speaking = fixture.passages.some((p) => ms >= (p.words[0][1] as number) && ms < (p.words[p.words.length - 1][2] as number));
    emit("note-level", speaking ? 0.02 + Math.random() * 0.12 : 0.002);
    // Like speechkit: the held audio is transcribed once the engine is ready.
    if (recorder.waitingForEngine) return;
    const lines = fixturePassages(ms).map((p) => ({ startMs: p.startMs, text: p.text }));
    const next = fixture.passages.find((p) => (p.words[0][1] as number) <= ms && (p.words[p.words.length - 1][2] as number) > ms);
    const partial = next ? next.words.filter((w) => (w[1] as number) <= ms).map((w) => w[0]).join(" ") : "";
    livePayload = { id: recorder.id, lines, partial, partialStartMs: next ? (next.words[0][1] as number) : null };
    emit("note-live", livePayload);
  }, 100);
}

const STEPS = ["Saving the recording", ...(mockCaps ? ["Finding who spoke when", "Timing every word"] : []), "Writing a summary"];

function finishRecorder() {
  const durationMs = recElapsed();
  const id = `n${Date.now()}`;
  const title = recorder.title;
  const marks = recorder.marks;
  clearInterval(recTimer);
  let step = 0;
  const show = () => setRecorder({ phase: "finishing", runningSince: null, elapsedMs: durationMs, steps: STEPS.map((label, i) => ({ label, state: i < step ? "done" : i === step ? "now" : "todo" })) });
  show();
  recTimer = setInterval(() => {
    step += 1;
    if (step < STEPS.length) return show();
    clearInterval(recTimer);
    notes.unshift(makeNote(id, title === "Untitled note" ? "Launch plan sync" : title, Date.now(), durationMs, marks));
    emit("notes-changed");
    setRecorder({ ...idle, saved: id });
  }, 700);
}

function noteCommand(cmd: string, a: Record<string, unknown>): unknown {
  const note = notes.find((n) => n.id === a.id);
  switch (cmd) {
    case "notes_list":
      return structuredClone(notes);
    case "note_recorder_state":
      return recorder;
    case "note_live":
      return recorder.phase === "recording" || recorder.phase === "paused" ? livePayload : null;
    case "new_voice_note":
      if (recorder.phase === "idle") {
        const waiting = params.get("loading") === "1";
        setRecorder({ ...idle, phase: "recording", id: `rec${Date.now()}`, title: "Untitled note", runningSince: Date.now(), waitingForEngine: waiting });
        if (waiting) setTimeout(() => setRecorder({ waitingForEngine: false }), 4000);
        runRecorder();
      }
      emit("navigate", "notes:new");
      return null;
    case "note_pause":
      return setRecorder({ phase: "paused", elapsedMs: recElapsed(), runningSince: null });
    case "note_resume":
      return setRecorder({ phase: "recording", runningSince: Date.now() });
    case "note_mark":
      return setRecorder({ marks: [...recorder.marks, recElapsed()] });
    case "note_discard":
      clearInterval(recTimer);
      return setRecorder({ ...idle });
    case "note_stop":
      return finishRecorder();
    case "note_rename":
      if (note) note.title = a.title as string;
      else if (recorder.id === a.id) setRecorder({ title: a.title as string });
      break;
    case "note_set_marks":
      if (note) note.marks = (a.marks as number[]).sort((x, y) => x - y);
      break;
    case "note_set_action":
      if (note) note.actions[a.index as number].done = a.done as boolean;
      break;
    case "note_delete":
      notes.splice(notes.indexOf(note!), 1);
      break;
    case "note_summarize":
      if (note) note.summary = "A short note.";
      break;
    case "note_text":
      return `# ${note?.title}\n\n${note?.passages.map((p) => p.text).join("\n\n")}`;
    case "note_export":
      console.info("note_export", a);
      return null;
    default:
      return undefined;
  }
  emit("notes-changed");
  return null;
}

// --- Transcripts ----------------------------------------------------------

/** The fixture played `times` over, one after another, as a longer file. */
function longPassages(times: number): Passage[] {
  const out: Passage[] = [];
  for (let k = 0; k < times; k++) {
    const offset = k * 24_000;
    for (const p of fixturePassages(24_000)) {
      out.push({
        ...p,
        startMs: p.startMs + offset,
        endMs: p.endMs + offset,
        words: p.words?.map((w) => ({ ...w, startMs: w.startMs + offset, endMs: w.endMs + offset })) ?? null,
        original: null,
      });
    }
  }
  return out;
}

function makeDoc(id: string, name: string, createdAt: number, times: number, saved: string[]): TranscriptDoc {
  const passages = longPassages(times);
  return {
    id,
    name,
    source: silence(times * 24_000),
    sourceExists: true,
    createdAt,
    durationMs: times * 24_000,
    engine: { name: "sense-voice-zh-en-ja-ko-yue-int8-2024-07-17", kind: "SenseVoice", onDevice: true },
    speakers: mockCaps ? [{ name: "Speaker 1" }, { name: "Speaker 2" }] : [],
    saved,
    passages,
  };
}

const docs: TranscriptDoc[] =
  params.get("transcripts") === "0"
    ? []
    : [
        makeDoc("t1", "team-sync-0926", now - 2 * 3_600_000, 10, ["team-sync-0926.srt", "team-sync-0926.vtt"]),
        makeDoc("t2", "customer-call-0921", now - 3 * 86_400_000, 6, ["customer-call-0921.srt"]),
        makeDoc("t3", "keynote-rehearsal", now - 20 * 86_400_000, 4, []),
      ];

let jobs: Job[] =
  params.get("transcripts") === "0"
    ? []
    : [
        { id: "j1", path: "/Users/me/Movies/podcast-episode-12.mp4", name: "podcast-episode-12.mp4", durationMs: 4_325_000, state: { kind: "running", stage: "Transcribing", progress: 0.62, secondsLeft: 240 } },
        { id: "j2", path: "/Users/me/Movies/interview-raw.mov", name: "interview-raw.mov", durationMs: null, state: { kind: "failed", error: "This file's audio can't be read (MOV is not supported yet)" } },
        { id: "j3", path: "/Users/me/Music/lecture-week-5.wav", name: "lecture-week-5.wav", durationMs: 5_460_000, state: { kind: "waiting" } },
      ];

function jobsChanged() {
  emit("transcript-jobs", jobs);
}

setInterval(() => {
  const running = jobs.find((j) => j.state.kind === "running");
  if (running?.state.kind === "running" && running.state.progress !== null) {
    running.state = { ...running.state, progress: Math.min(0.99, running.state.progress + 0.01), secondsLeft: Math.max(10, (running.state.secondsLeft ?? 0) - 6) };
    jobsChanged();
  }
}, 2000);

/** Rough cues for the preview; the app builds them in Rust (subtitles.rs). */
function mockCues(doc: TranscriptDoc, names: boolean): Cue[] {
  const cues: Cue[] = [];
  for (const p of doc.passages) {
    const who = names && p.speaker !== null && doc.speakers[p.speaker] ? `${doc.speakers[p.speaker].name}: ` : "";
    if (!p.words || p.original) {
      cues.push({ n: cues.length + 1, startMs: p.startMs, endMs: p.endMs, text: who + p.text, speaker: p.speaker, warning: null });
      continue;
    }
    let group: typeof p.words = [];
    const flush = () => {
      if (!group.length) return;
      cues.push({ n: cues.length + 1, startMs: group[0].startMs, endMs: group[group.length - 1].endMs, text: (cues.at(-1)?.speaker === p.speaker && cues.at(-1)!.endMs >= p.startMs ? "" : who) + group.map((w) => w.text).join(" "), speaker: p.speaker, warning: null });
      group = [];
    };
    for (const w of p.words) {
      if (group.length && (group.map((x) => x.text).join(" ").length + w.text.length > 42 || w.endMs - group[0].startMs > 7000)) flush();
      group.push(w);
    }
    flush();
  }
  return cues;
}

function transcriptCommand(cmd: string, a: Record<string, unknown>): unknown {
  const doc = docs.find((d) => d.id === a.id);
  const changed = () => {
    emit("transcripts-changed");
    return null;
  };
  switch (cmd) {
    case "transcript_jobs":
      return jobs;
    case "transcript_job_retry":
      jobs = jobs.map((j) => (j.id === a.id ? { ...j, state: { kind: "waiting" } } : j));
      jobsChanged();
      return null;
    case "transcript_job_cancel":
      jobs = jobs.filter((j) => j.id !== a.id);
      jobsChanged();
      return null;
    case "transcripts_list":
      return docs.map(({ passages: _, ...summary }) => summary);
    case "transcript_get":
      if (!doc) throw "the transcript is gone";
      return structuredClone(doc);
    case "transcripts_search": {
      const q = (a.query as string).toLowerCase();
      return docs.flatMap((d) => {
        const matches = d.passages.filter((p) => p.text.toLowerCase().includes(q));
        if (!matches.length) return [];
        const p = matches[0];
        const at = p.text.toLowerCase().indexOf(q);
        return [{ id: d.id, name: d.name, count: matches.length, atMs: p.startMs, before: "…" + p.text.slice(Math.max(0, at - 24), at), match: p.text.slice(at, at + q.length), after: p.text.slice(at + q.length, at + q.length + 24) + "…" }];
      });
    }
    case "transcript_edit_passage": {
      const p = doc!.passages[a.index as number];
      p.original ??= p.text;
      p.text = a.text as string;
      p.words = null;
      return changed();
    }
    case "transcript_rename_speaker":
      doc!.speakers[a.index as number].name = a.name as string;
      return changed();
    case "transcript_replace_all": {
      let n = 0;
      const re = new RegExp((a.find as string).replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi");
      for (const p of doc!.passages) {
        const hits = p.text.match(re)?.length ?? 0;
        if (!hits) continue;
        n += hits;
        p.original ??= p.text;
        p.text = p.text.replace(re, a.replace as string);
        p.words = null;
      }
      changed();
      return n;
    }
    case "transcript_delete":
      docs.splice(docs.indexOf(doc!), 1);
      return changed();
    case "transcript_cues":
      return mockCues(doc!, a.speakerNames as boolean);
    case "transcript_export":
      console.info("transcript_export", a);
      return null;
    case "update_transcript_settings":
      Object.assign(snapshot.settings.transcripts, a.patch as Partial<TranscriptSettings>);
      setTimeout(() => emit("state-changed"), 0);
      return null;
    default:
      return undefined;
  }
}

/** Commands that change settings update the mock and tell the windows. */
function changed() {
  setTimeout(() => emit("state-changed"), 0);
}

mockIPC(
  async (cmd, args) => {
    const a = args as Record<string, unknown>;
    if (cmd === "get_state") return structuredClone(snapshot);
    if (cmd === "plugin:dialog|open") {
      console.info("plugin:dialog|open", a);
      return null;
    }
    if (cmd === "dictionary_save") {
      const entry = a.entry as DictionaryEntry;
      const words = snapshot.settings.dictionary;
      if (!entry.id) entry.id = String(Date.now());
      const at = words.findIndex((w) => w.id === entry.id);
      if (at >= 0) words[at] = entry;
      else words.push(entry);
      changed();
      return entry.id;
    }
    if (cmd === "dictionary_remove") {
      snapshot.settings.dictionary = snapshot.settings.dictionary.filter((w) => w.id !== a.id);
      changed();
      return null;
    }
    if (cmd === "save_api_key" || cmd === "delete_api_key") {
      snapshot.keys[a.provider as Provider] = cmd === "save_api_key";
      changed();
      return null;
    }
    if (cmd === "test_polish_connection") {
      await new Promise((r) => setTimeout(r, 700));
      return null;
    }
    if (cmd === "update_polish") {
      Object.assign(snapshot.settings.polish, a.patch as Partial<PolishSettings>);
      changed();
      return null;
    }
    if (cmd === "set_app_tone") {
      const { target, tone } = a as { target: string; tone: Tone | null };
      const tones = snapshot.settings.polish.tones;
      const at = tones.findIndex((t) => t.app.toLowerCase() === target.toLowerCase());
      if (at >= 0 && tone) tones[at] = { ...tones[at], tone };
      else if (at >= 0) tones.splice(at, 1);
      else if (tone) tones.push({ app: target, tone });
      changed();
      return null;
    }
    if (cmd === "polish_preview") {
      await new Promise((r) => setTimeout(r, 700));
      return "Can you rerun the nightly with three test threads? Models aborted again.";
    }
    if (cmd === "pause_dictation") {
      const minutes = a.minutes as number | null;
      snapshot.paused = { until: minutes ? Date.now() + minutes * 60_000 : null };
      changed();
      return null;
    }
    if (cmd === "resume_dictation") {
      snapshot.paused = null;
      changed();
      return null;
    }
    if (cmd === "bind_shortcut" && snapshot.desktop) {
      await new Promise((r) => setTimeout(r, 600));
      snapshot.desktop.shortcut.bound = true;
      changed();
      return null;
    }
    if (cmd === "set_typing" && snapshot.desktop) {
      await new Promise((r) => setTimeout(r, 400));
      snapshot.desktop.typing = a.method as "extension" | "portal" | "clipboard";
      if (a.method === "portal") snapshot.desktop.portalAllowed = true;
      changed();
      return null;
    }
    if (cmd === "install_extension" && snapshot.desktop) {
      await new Promise((r) => setTimeout(r, 600));
      snapshot.desktop.extension = "installed";
      changed();
      return null;
    }
    if (cmd === "set_autostart") {
      snapshot.autostart = a.on as boolean;
      changed();
      return null;
    }
    if (cmd === "set_key_test") {
      // ?os=windows&w=setup: a fake press of the talk key while testing it.
      if (a.on) setTimeout(() => emit("hotkey", "down").then(() => setTimeout(() => emit("hotkey", "up"), 900)), 1500);
      return null;
    }
    if (cmd === "set_preferences") {
      Object.assign(snapshot.settings, a.prefs as object);
      const hotkey = (a.prefs as { hotkey?: keyof typeof KEY_NAMES }).hotkey;
      if (hotkey) snapshot.hotkeyName = KEY_NAMES[hotkey];
      changed();
      return null;
    }
    if (cmd === "history_list") return history;
    if (cmd === "audio_extensions") return ["aac", "flac", "m4a", "mka", "mkv", "mp3", "mp4", "oga", "ogg", "wav"];
    if (cmd.startsWith("note") || cmd === "new_voice_note") return noteCommand(cmd, a);
    if (cmd === "open_main") {
      emit("navigate", a.page as string);
      return null;
    }
    if (cmd === "transcribe_files") {
      const audio = (a.paths as string[]).filter((p) => /\.(aac|flac|m4a|mka|mkv|mp3|mp4|oga|ogg|wav)$/i.test(p));
      if (!audio.length) {
        emit("navigate", "transcripts:nothing");
        throw "None of these files can be transcribed";
      }
      for (const path of audio) {
        const name = path.split("/").pop()!;
        jobs.push({ id: `j${Date.now()}${name}`, path, name, durationMs: null, state: { kind: "waiting" } });
      }
      jobsChanged();
      emit("navigate", "transcripts:added");
      return null;
    }
    if (cmd.startsWith("transcript") || cmd === "update_transcript_settings") return transcriptCommand(cmd, a);
    if (cmd === "list_microphones") return [{ name: "MacBook Pro Microphone", isDefault: true }, { name: "AirPods Pro", isDefault: false }, { name: "BlackHole 2ch", isDefault: false }];
    return null;
  },
  { shouldMockEvents: true },
);
// Recordings play from blob URLs in the preview.
(window as unknown as { __TAURI_INTERNALS__: { convertFileSrc: (p: string) => string } }).__TAURI_INTERNALS__.convertFileSrc = (path) => path;

const pills: Record<string, PillView> = {
  idle: { kind: "idle" },
  listening: { kind: "listening", token: 1, startedAt: now - 4000, context: "Mail", live: true },
  handsFree: { kind: "handsFree", token: 1, startedAt: now - 102_000, limitMs: 300_000, context: "Microsoft PowerPoint · Cloud", live: true },
  transcribing: { kind: "transcribing", label: "Transcribing" },
  polishing: { kind: "polishing" },
  inserted: { kind: "inserted", label: "38 words", canRaw: true },
  copied: { kind: "copied", label: "No text field focused · copied to clipboard", hint: "⌘V to paste" },
  failed: { kind: "failed", message: "Connection lost · audio kept", detail: "", retryable: true, alternative: "Use on-device" },
  hint: { kind: "hint", text: "Hold fn to dictate, or double-tap it" },
};

async function render() {
  const root = createRoot(document.getElementById("root")!);
  if (label === "pill") {
    const { Pill } = await import("./windows/Pill");
    document.body.style.background = "#CEC8BA";
    root.render(
      <div style={{ width: 820, height: 120 }}>
        <Pill />
      </div>,
    );
    const view = pills[params.get("state") ?? "listening"];
    setTimeout(() => {
      emit("pill-state", view);
      if (view.kind === "listening" || view.kind === "handsFree") {
        emit("pill-partial", [1, "so the plan is to ship on Thursday"]);
        let t = 0;
        setInterval(() => emit("pill-level", 0.02 + Math.abs(Math.sin(t++ * 0.7)) * 0.12), 60);
      }
    }, 300);
  } else if (label === "setup") {
    const { Setup } = await import("./windows/Setup");
    document.body.style.background = "#CEC8BA";
    root.render(
      <div style={{ width: 1040, height: 700, margin: 24, borderRadius: 8, overflow: "hidden", boxShadow: "0 0 0 1px rgba(0,0,0,.12)" }}>
        <Setup />
      </div>,
    );
  } else if (label === "popover") {
    const { Popover } = await import("./windows/Popover");
    document.body.style.background = "#CEC8BA";
    root.render(<div style={{ width: 360 }}><Popover /></div>);
  } else {
    const { MainWindow } = await import("./windows/MainWindow");
    root.render(<MainWindow />);
    const page = params.get("page");
    if (page) setTimeout(() => emit("navigate", page), 300);
  }
}
render();
