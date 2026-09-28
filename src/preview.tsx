// Dev only: renders Viary's windows in a browser against a mocked backend,
// to compare them with the design. Open /preview.html?w=main|popover|pill.

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
import type { HistoryItem, PillView, Snapshot } from "./lib/ipc";

const params = new URLSearchParams(location.search);
const label = params.get("w") ?? "main";
mockWindows(label);
document.documentElement.dataset.window = label;

const now = Date.now();
const snapshot: Snapshot = {
  settings: {
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
    hotkey: "fn",
    keepRecordingsDays: 7,
    openai: { baseUrl: "https://api.openai.com/v1", model: "", mode: "file" },
    dashscope: { model: "", region: "china" },
  },
  engine: {
    active: { id: "local:2", name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17", kind: "SenseVoice", onDevice: true, live: false, punctuation: "native", languageOverride: true },
    loading: null,
    failed: null,
    error: null,
  },
  keys: { openAi: true, dashScope: false },
  permissions: { accessibility: true, inputMonitoring: true },
  hotkeyActive: true,
  hotkeyName: "fn",
  pill: { kind: "idle" },
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

mockIPC(
  (cmd) => {
    if (cmd === "get_state") return snapshot;
    if (cmd === "history_list") return history;
    if (cmd === "list_microphones") return [{ name: "MacBook Pro Microphone", isDefault: true }, { name: "AirPods Pro", isDefault: false }, { name: "BlackHole 2ch", isDefault: false }];
    return null;
  },
  { shouldMockEvents: true },
);

const pills: Record<string, PillView> = {
  idle: { kind: "idle" },
  listening: { kind: "listening", token: 1, startedAt: now - 4000, context: "Mail", live: true },
  transcribing: { kind: "transcribing", label: "Transcribing" },
  inserted: { kind: "inserted", label: "38 words", canRaw: true },
  copied: { kind: "copied", label: "No text field focused · copied to clipboard", hint: "⌘V to paste" },
  failed: { kind: "failed", message: "Connection lost · audio kept", detail: "", retryable: true, alternative: "Use on-device" },
  hint: { kind: "hint", text: "Hold fn while you speak" },
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
      if (view.kind === "listening") {
        emit("pill-partial", [1, "so the plan is to ship on Thursday"]);
        let t = 0;
        setInterval(() => emit("pill-level", 0.02 + Math.abs(Math.sin(t++ * 0.7)) * 0.12), 60);
      }
    }, 300);
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
