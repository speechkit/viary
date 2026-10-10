// The setup window on Windows (WinOnboarding artboard) and Ubuntu
// (LinuxShortcut): from the microphone to a first dictation. Viary runs in the tray all along; closing the window leaves
// setup for the next launch.

import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useCallback, useEffect, useRef, useState } from "react";
import { ErrorBanner, Switch } from "../components/Controls";
import { Icon, Mark } from "../components/Icons";
import { AddModel, chooseVad, useAddModel } from "../pages/Engine";
import { api, engineName, errorText, formatBytes, useHotkey, useMicLevel, usePillView, useSnapshot, shortcutTrouble, type Desktop, type Hotkey, type PillView, type Snapshot } from "../lib/ipc";
import { PLATFORM, THIS_COMPUTER } from "../lib/platform";

/** GNOME's look and steps: Ubuntu. Windows otherwise. */
const GNOME = PLATFORM === "linux";

const STEPS = GNOME
  ? ["Microphone", "Talk shortcut", "Voice engine", "Try it"]
  : ["Microphone access", "Talk key", "Voice engine", "Keep it in the tray", "Try it"];

/** What the sidebar says under the steps, for each step (Windows). */
const NOTES = [
  "Windows can turn the microphone off for every desktop app at once. Viary checks that switch and opens its page if it is off.",
  "Viary watches only the talk key. Other keys go straight to the app you are typing in.",
  "On-device models keep audio on this PC. Cloud engines send it to the provider you choose.",
  "Closing the main window keeps Viary in the tray. Quit it from the tray menu.",
  "Everything you dictate is kept in History, where you can copy it again.",
];

const btn = GNOME
  ? "inline-flex h-[34px] items-center justify-center gap-2 rounded-md bg-black/8 px-4 text-sm font-semibold text-[#1E1E1E] hover:bg-black/12 disabled:opacity-40"
  : "inline-flex h-10 items-center justify-center gap-2 rounded-md border border-edge bg-white px-5 text-sm font-medium text-ink disabled:opacity-40";
const btnPrimary = GNOME
  ? "inline-flex h-[34px] items-center justify-center gap-2 rounded-md bg-[#C34113] px-4 text-sm font-semibold text-white disabled:opacity-40"
  : "inline-flex h-10 items-center justify-center gap-2 rounded-md border border-ink bg-ink px-5 text-sm font-medium text-white disabled:opacity-40";
/** A GNOME boxed list: white rows in a rounded card. */
const boxed = "overflow-hidden rounded-xl bg-white shadow-[0_0_0_1px_rgba(0,0,0,0.1)]";
const row = "flex min-h-[54px] items-center gap-3 border-b border-black/8 px-4 text-[15px] last:border-b-0";
const sub = "text-[13px] text-[#5E5E5E]";
const cap = "text-sm leading-normal text-muted";

/** A key cap as the design draws it in setup: taller than Kbd, mono. */
function Key({ children, dark = false }: { children: React.ReactNode; dark?: boolean }) {
  return (
    <span
      className={
        "inline-flex h-[46px] min-w-12 items-center justify-center rounded-lg border border-b-4 px-3 font-mono text-sm " +
        (dark ? "border-ink bg-ink text-white" : "border-stone bg-card text-ink")
      }
    >
      {children}
    </span>
  );
}

/** Hides setup, which runs again next time. Its page stays loaded, so the
 *  key and microphone tests are ended here rather than on unmount. */
function closeSetup() {
  api.setKeyTest(false).catch(() => {});
  api.micTest(false).catch(() => {});
  getCurrentWebviewWindow().hide();
}

function TitleBar() {
  const window = getCurrentWebviewWindow();
  const caption = "flex h-9 w-[46px] items-center justify-center text-ink hover:bg-ink/8";
  return (
    <div data-tauri-drag-region className="flex h-9 shrink-0 items-center bg-sand pl-3.5 text-xs">
      <span aria-hidden="true" className="mr-2.5 inline-flex size-4 items-center justify-center rounded bg-ink">
        <Mark size={10} />
      </span>
      <span data-tauri-drag-region className="grow">
        Set up Viary
      </span>
      <button type="button" aria-label="Minimize" className={caption} onClick={() => window.minimize()}>
        <Icon name="minus" size={14} />
      </button>
      <button type="button" aria-label="Close" className={caption + " hover:bg-[#C42B1C] hover:text-white"} onClick={closeSetup}>
        <Icon name="close" size={14} />
      </button>
    </div>
  );
}

/** GNOME's header bar: the title centered, a round close button. */
function HeaderBar() {
  return (
    <div data-tauri-drag-region className="flex h-[46px] shrink-0 items-center border-b border-[#DADADA] bg-[#EBEBEB] pr-2.5 pl-4">
      <span data-tauri-drag-region className="ml-6 grow text-center text-sm font-bold">
        Set up Viary
      </span>
      <button
        type="button"
        aria-label="Close"
        onClick={closeSetup}
        className="flex size-6 items-center justify-center rounded-full bg-black/8 hover:bg-black/12"
      >
        <Icon name="close" size={12} strokeWidth={2.2} />
      </button>
    </div>
  );
}

/** GNOME's sidebar rows, the current one shaded. */
function GnomeSteps({ current }: { current: number }) {
  return (
    <ol className="m-0 flex list-none flex-col gap-0.5 p-0">
      {STEPS.map((label, i) => {
        const done = i < current;
        const now = i === current;
        return (
          <li
            key={label}
            aria-current={now ? "step" : undefined}
            className={
              "flex h-10 items-center gap-3 rounded-lg px-3 text-sm " +
              (now ? "bg-black/7 font-bold text-[#1E1E1E]" : i > current ? "text-[#5E5E5E]" : "text-[#1E1E1E]")
            }
          >
            <span
              className={
                "flex size-[22px] items-center justify-center rounded-full text-xs " +
                (done ? "bg-[#26734D] text-white" : now ? "bg-[#1E1E1E] text-white" : "border-[1.5px] border-[#B0B0B0] text-[#5E5E5E]")
              }
            >
              {done ? <Icon name="check" size={12} strokeWidth={2.4} /> : i + 1}
            </span>
            {label}
          </li>
        );
      })}
    </ol>
  );
}

function Steps({ current }: { current: number }) {
  return (
    <ol className="m-0 flex list-none flex-col gap-[18px] p-0">
      {STEPS.map((label, i) => {
        const done = i < current;
        const now = i === current;
        return (
          <li
            key={label}
            aria-current={now ? "step" : undefined}
            className={"flex items-center gap-3 text-sm " + (now ? "font-semibold" : "") + (i > current ? " text-faint" : " text-ink")}
          >
            <span
              className={
                "flex size-[26px] items-center justify-center rounded-full text-xs " +
                (done ? "bg-teal text-white" : now ? "bg-ink text-white" : "border-[1.5px] border-stone text-faint")
              }
            >
              {done ? <Icon name="check" size={14} strokeWidth={2} /> : i + 1}
            </span>
            {label}
          </li>
        );
      })}
    </ol>
  );
}

function Heading({ step, title, children }: { step: number; title: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-2">
      {!GNOME && (
        <span className="text-[13px] text-faint">
          Step {step + 1} of {STEPS.length}
        </span>
      )}
      <h1 className={"m-0 font-serif leading-[1.1] font-normal " + (GNOME ? "text-4xl" : "text-[38px]")}>{title}</h1>
      <span className={GNOME ? "text-[15px] leading-normal text-[#5E5E5E]" : cap}>{children}</span>
    </div>
  );
}

/** A status row: a check when done, an amber dot when not. */
function Status({ ok, title, text, action }: { ok: boolean; title: string; text: string; action?: React.ReactNode }) {
  return (
    <div className="flex items-center gap-4 rounded-[10px] border border-line bg-white px-[18px] py-4">
      {ok ? (
        <Icon name="check" size={22} strokeWidth={2} className="shrink-0 text-teal" />
      ) : (
        <span aria-hidden="true" className="mx-1.5 size-2.5 shrink-0 rounded-full bg-amber" />
      )}
      <div className="flex grow flex-col gap-0.5">
        <span className="text-[15px] font-semibold">{title}</span>
        <span className="text-[13px] leading-[1.45] text-muted">{text}</span>
      </div>
      {action}
    </div>
  );
}

function MicrophoneStep({ s }: { s: Snapshot }) {
  const allowed = s.permissions.microphone !== false;
  const [mics, setMics] = useState<{ name: string; isDefault: boolean }[]>([]);
  useEffect(() => {
    api.microphones().then(setMics, () => setMics([]));
  }, [allowed]);
  return (
    <>
      <Heading step={0} title="Let Viary hear you">
        Viary listens only while you hold your talk key, and keeps no audio unless you ask it to.
      </Heading>
      <Status
        ok={allowed}
        title={allowed ? "Desktop apps can use the microphone" : "Windows has the microphone off for desktop apps"}
        text={
          allowed
            ? "“Let desktop apps access your microphone” is on."
            : "Turn on “Let desktop apps access your microphone”, then come back here."
        }
        action={
          !allowed && (
            <button type="button" className={btn} onClick={() => api.requestPermission("microphone")}>
              Open Settings
            </button>
          )
        }
      />
      {allowed && mics.length > 0 && (
        <label className="flex flex-col gap-2">
          <span className="text-[13px] text-faint">Microphone</span>
          <select
            value={s.settings.microphone ?? ""}
            onChange={(e) => api.setPreferences({ microphone: e.target.value || null })}
            className="h-10 rounded-md border border-edge bg-white px-3 text-sm"
          >
            <option value="">Default ({mics.find((m) => m.isDefault)?.name ?? "system"})</option>
            {mics.map((m) => (
              <option key={m.name} value={m.name}>
                {m.name}
              </option>
            ))}
          </select>
        </label>
      )}
    </>
  );
}

const SEGMENTS = 24;

/** Microphone RMS to 0..1: -60 dB is silence, -15 dB is a raised voice. */
function loudness(level: number): number {
  const db = 20 * Math.log10(Math.max(level, 1e-5));
  return Math.min(1, Math.max(0, (db + 60) / 45));
}

/** Speech, not room noise: about -35 dB. */
const SPEECH = 0.55;
/** How long silence lasts before the step suggests Sound settings. */
const QUIET_FOR = 6000;

/** The microphone on GNOME: pick the device, and see it hear you. */
function GnomeMicrophoneStep({ s }: { s: Snapshot }) {
  const [mics, setMics] = useState<{ name: string; isDefault: boolean }[]>([]);
  useEffect(() => {
    api.microphones().then(setMics, () => setMics([]));
  }, []);
  const { level, error } = useMicLevel(s.settings.microphone);
  const [heard, setHeard] = useState(false);
  const [quiet, setQuiet] = useState(false);
  const loud = loudness(level);
  // A new microphone has not been heard yet.
  useEffect(() => {
    setHeard(false);
    setQuiet(false);
    const timer = setTimeout(() => setQuiet(true), QUIET_FOR);
    return () => clearTimeout(timer);
  }, [s.settings.microphone]);
  useEffect(() => {
    if (loud >= SPEECH) setHeard(true);
  }, [loud]);
  const lit = Math.round(loud * SEGMENTS);
  return (
    <>
      <Heading step={0} title="Let Viary hear you">
        Viary listens only while you use your talk shortcut, and keeps no audio unless you ask it to.
      </Heading>
      <div className={boxed}>
        <label className={row}>
          <span className="grow">Microphone</span>
          <select
            value={s.settings.microphone ?? ""}
            onChange={(e) => api.setPreferences({ microphone: e.target.value || null })}
            className="h-[34px] max-w-[320px] truncate rounded-md bg-black/8 px-3 text-sm font-semibold"
          >
            <option value="">Default ({mics.find((m) => m.isDefault)?.name ?? "system"})</option>
            {mics.map((m) => (
              <option key={m.name} value={m.name}>
                {m.name}
              </option>
            ))}
          </select>
        </label>
        <div className={row + " py-3"}>
          <div className="flex grow flex-col">
            <span>Input level</span>
            <span className={sub}>Say something; the bars should move.</span>
          </div>
          <div
            role="meter"
            aria-label="Input level"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(loud * 100)}
            className="flex h-4 items-center gap-[3px]"
          >
            {Array.from({ length: SEGMENTS }, (_, i) => (
              <span key={i} className={"h-4 w-[5px] rounded-sm " + (i < lit ? "bg-[#26734D]" : "bg-black/10")} />
            ))}
          </div>
        </div>
      </div>
      {error ? (
        <div className="rounded-xl bg-[#FDF3D6] px-4 py-3 text-[13px] leading-normal text-[#5C4400]">
          Viary cannot open this microphone: {error}
        </div>
      ) : heard ? (
        <span role="status" className="flex items-center gap-2 text-sm text-[#26734D]">
          <Icon name="check" size={18} strokeWidth={2.2} />
          Viary hears you.
        </span>
      ) : (
        quiet && (
          <div role="status" className="flex items-center gap-3 rounded-xl bg-[#FDF3D6] px-4 py-3 text-[13px] leading-normal text-[#5C4400]">
            <span className="grow">No voice yet. Check the input device and its volume in Sound settings.</span>
            <button type="button" className={btn} onClick={() => api.requestPermission("microphone")}>
              Sound Settings…
            </button>
          </div>
        )
      )}
    </>
  );
}

const KEYS: { key: Hotkey; caps: string[]; label: string; note: string }[] = [
  { key: "rightAlt", caps: ["Right Alt"], label: "Right Alt", note: "Recommended" },
  { key: "ctrlWin", caps: ["Ctrl", "Win"], label: "Ctrl + Win", note: "Left side, two keys" },
];

/** Press the talk key to see Viary hear it. While shown, the key only
 *  reports itself: no dictation, no hints. */
export function KeyTest({ name }: { name: string }) {
  useEffect(() => {
    api.setKeyTest(true);
    // A hidden window keeps its page: the test follows the window's focus,
    // so the key goes back to dictating once the window is closed or left.
    const unlisten = getCurrentWebviewWindow().onFocusChanged(({ payload: focused }) => {
      api.setKeyTest(focused);
    });
    return () => {
      unlisten.then((stop) => stop());
      api.setKeyTest(false);
    };
  }, []);
  const [pressed, setPressed] = useState<number | null>(null);
  const [held, setHeld] = useState<number | null>(null);
  useHotkey(
    useCallback((down: boolean) => {
      if (down) {
        setPressed(Date.now());
        setHeld(null);
      } else {
        setPressed((at) => {
          if (at !== null) setHeld(Date.now() - at);
          return null;
        });
      }
    }, []),
  );
  const tested = held !== null;
  const title = pressed !== null ? "Held… now let go" : tested ? "Viary hears it" : "Press it now to test";
  const text = tested ? `Detected: ${name} held for ${(held / 1000).toFixed(1)} s` : `Hold ${name} for a moment, then release it.`;
  return (
    <div role="status" className="flex items-center gap-4 rounded-[10px] border border-dashed border-stone bg-white px-[18px] py-4">
      <Key dark>{name}</Key>
      <div className="flex grow flex-col gap-0.5">
        <span className="text-[15px] font-semibold">{title}</span>
        <span className="text-[13px] text-muted">{text}</span>
      </div>
      {tested && <Icon name="check" size={22} strokeWidth={2} className="shrink-0 text-teal" />}
    </div>
  );
}

function TalkKeyStep({ s }: { s: Snapshot }) {
  const name = KEYS.find((k) => k.key === s.settings.hotkey)?.label ?? s.hotkeyName;
  // On AltGr layouts Right Alt types characters: Ctrl + Win is the one to use.
  const altGr = s.keyboard?.altGr ?? false;
  const note = (k: (typeof KEYS)[number]) =>
    altGr ? (k.key === "ctrlWin" ? "Recommended for your keyboard" : "Types é, @… on your keyboard") : k.note;
  return (
    <>
      <Heading step={1} title="Pick your talk key">
        Hold it and speak, let go and your words appear. Most Windows keyboards don’t let apps see Fn, so Viary uses a
        key it can hear.
      </Heading>
      <div role="radiogroup" aria-label="Talk key" className="grid grid-cols-3 gap-3">
        {KEYS.map((k) => {
          const on = s.settings.hotkey === k.key;
          return (
            <button
              key={k.key}
              type="button"
              role="radio"
              aria-checked={on}
              onClick={() => api.setPreferences({ hotkey: k.key })}
              className={
                "flex flex-col items-center gap-2.5 rounded-[10px] bg-white px-2.5 py-[18px] text-sm font-medium " +
                (on ? "border-2 border-ink" : "border border-line")
              }
            >
              <span className="flex gap-1">
                {k.caps.map((c) => (
                  <Key key={c}>{c}</Key>
                ))}
              </span>
              {k.label}
              <small className="text-xs font-normal text-faint">{note(k)}</small>
            </button>
          );
        })}
        <button
          type="button"
          role="radio"
          aria-checked={false}
          disabled
          className="flex flex-col items-center gap-2.5 rounded-[10px] border border-line bg-white px-2.5 py-[18px] text-sm font-medium opacity-60"
        >
          <Key>…</Key>
          Custom
          <small className="text-xs font-normal text-faint">Coming soon</small>
        </button>
      </div>
      {altGr && (
        <div className="rounded-[10px] border border-[#F0DDB6] bg-[#FFF6E6] px-4 py-3 text-[13px] leading-[1.45] text-[#6B4A00]">
          Your keyboard layout uses Right Alt as AltGr, for characters like é and @, so Viary suggests Ctrl + Win
          instead. Win + H stays with Windows voice typing.
        </div>
      )}
      <KeyTest key={name} name={name} />
      <div className="flex items-center gap-3 text-sm">
        <span className="grow">Start Viary when I sign in to Windows</span>
        <Switch on={s.autostart} label="Start Viary when I sign in to Windows" onChange={(on) => api.setAutostart(on)} />
      </div>
    </>
  );
}

function EngineStep({ s }: { s: Snapshot }) {
  const [error, setError] = useState("");
  const { active, loading } = s.engine;
  return (
    <>
      <Heading step={2} title="Choose how Viary listens">
        A speech model on {THIS_COMPUTER} keeps your voice there. Or use OpenAI or DashScope with your own API key.
      </Heading>
      <ErrorBanner error={error} onDismiss={() => setError("")} />
      {active || loading ? (
        <Status
          ok={!!active && !loading}
          title={loading ? "Loading the voice engine…" : active!.onDevice ? `On ${THIS_COMPUTER} · ${active!.name}` : `Cloud · ${active!.kind} · ${active!.name}`}
          text={loading ? "The first load takes a few seconds." : "Viary transcribes with this engine. You can change it any time."}
          action={
            <button type="button" className={btn} onClick={() => api.openMain("engine")}>
              Change…
            </button>
          }
        />
      ) : (
        <div className="grid grid-cols-2 gap-3">
          <AddModel onError={setError} />
          <button
            type="button"
            onClick={() => api.openMain("engine")}
            className="flex min-h-[172px] flex-col items-center justify-center gap-2 rounded-[14px] border border-dashed border-stone px-[18px] py-4 text-muted hover:border-ink hover:text-ink"
          >
            <Icon name="cloud" size={20} />
            <span className="text-sm font-medium">Use a cloud engine…</span>
            <span className="max-w-[220px] text-center text-xs leading-[1.45] text-faint">
              OpenAI or DashScope, with your API key. Opens Voice engine in Viary.
            </span>
          </button>
        </div>
      )}
    </>
  );
}

/** A GNOME radio: an orange ring when on. */
function Radio({ on }: { on: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={"size-[18px] shrink-0 rounded-full " + (on ? "border-[5px] border-[#C34113]" : "border-[1.5px] border-[#B0B0B0]")}
    />
  );
}

/** The voice engine on GNOME: the models added, adding one, the VAD file
 *  phrase-by-phrase models need, and the cloud, in GNOME's lists. */
function GnomeEngineStep({ s }: { s: Snapshot }) {
  const [error, setError] = useState("");
  const adding = useAddModel(setError);
  const { active, loading, failed } = s.engine;
  const chosen = loading ?? failed ?? active?.id ?? null;
  const family = (id: string) => s.families.find(([model]) => model === id)?.[1];
  const local = s.settings.localModels.find((m) => `local:${m.id}` === chosen);
  const needsVad = !!local && !!family(local.id)?.needsVad && !s.settings.vadModel;
  return (
    <>
      <Heading step={2} title="Choose how Viary listens">
        A speech model on this computer keeps your voice here. Or use OpenAI or DashScope with your own API key.
      </Heading>
      <span className="-mb-3 text-[13px] font-bold text-[#5E5E5E]">On this computer</span>
      <div role="radiogroup" aria-label="Speech model" className={boxed}>
        {s.settings.localModels.map((m) => {
          const id = `local:${m.id}`;
          return (
            <button
              key={m.id}
              type="button"
              role="radio"
              aria-checked={chosen === id}
              onClick={() => api.setActiveEngine(id).catch((e) => setError(errorText(e)))}
              className={row + " w-full py-3 text-left"}
            >
              <Radio on={chosen === id} />
              <span className="flex min-w-0 grow flex-col">
                <span className="truncate">{m.name}</span>
                <span className={sub}>
                  {family(m.id)?.label ?? m.family} · {formatBytes(m.sizeBytes)}
                </span>
              </span>
            </button>
          );
        })}
        {adding.found ? (
          <div className={row + " flex-col items-stretch gap-2 py-3"}>
            <span>
              {adding.found.name} <span className={sub}>· {formatBytes(adding.found.sizeBytes)}</span>
            </span>
            <span className={sub}>The files do not say which kind of model this is; choose one.</span>
            <div role="radiogroup" aria-label="Model family" className="flex flex-col gap-1">
              {adding.found.families.map((f) => (
                <button
                  key={f.id}
                  type="button"
                  role="radio"
                  aria-checked={adding.family === f.id}
                  onClick={() => adding.setFamily(f.id)}
                  className="flex items-center gap-3 py-1 text-left text-sm"
                >
                  <Radio on={adding.family === f.id} />
                  {f.label}
                </button>
              ))}
            </div>
            <div className="flex justify-end gap-2">
              <button type="button" className={btn} onClick={adding.cancel}>
                Cancel
              </button>
              <button type="button" className={btnPrimary} disabled={adding.busy} onClick={() => adding.add(adding.found!, adding.family)}>
                Add and use
              </button>
            </div>
          </div>
        ) : (
          <div className={row}>
            <span className="flex grow flex-col">
              <span>{s.settings.localModels.length ? "Another model" : "A sherpa-onnx model folder"}</span>
              <span className={sub}>Unpacked from the sherpa-onnx releases. Viary detects its layout.</span>
            </span>
            <button type="button" className={btn} disabled={adding.busy} onClick={adding.pick}>
              Add Folder…
            </button>
          </div>
        )}
      </div>
      {needsVad && (
        <div className="flex items-center gap-3 rounded-xl bg-[#FDF3D6] px-4 py-3 text-[13px] leading-normal text-[#5C4400]">
          <span className="grow">
            {local!.name} transcribes phrase by phrase and needs silero_vad.onnx to find the pauses. Viary finds it next
            to the model folder; otherwise choose it.
          </span>
          <button type="button" className={btn} onClick={() => chooseVad(setError)}>
            Choose File…
          </button>
        </div>
      )}
      <span className="-mb-3 text-[13px] font-bold text-[#5E5E5E]">Cloud</span>
      <div className={boxed}>
        <div className={row}>
          <span className="flex grow flex-col">
            <span>OpenAI or DashScope</span>
            <span className={sub}>With your own API key, kept in the GNOME keyring. Audio goes to the provider.</span>
          </span>
          <button type="button" className={btn} onClick={() => api.openMain("engine")}>
            Set Up…
          </button>
        </div>
      </div>
      {loading ? (
        <span role="status" className={sub}>
          Loading {engineName(s, loading)}… the first load takes a few seconds.
        </span>
      ) : failed && s.engine.error ? (
        <div role="alert" className="rounded-xl bg-[#FBE9E1] px-4 py-3 text-[13px] leading-normal text-[#A33B12]">
          {engineName(s, failed)} did not load: {s.engine.error}
        </div>
      ) : (
        active && (
          <span role="status" className="flex items-center gap-2 text-sm text-[#26734D]">
            <Icon name="check" size={18} strokeWidth={2.2} />
            Ready: {active.name}
          </span>
        )
      )}
      <ErrorBanner error={error} onDismiss={() => setError("")} />
    </>
  );
}

function TrayStep({ s }: { s: Snapshot }) {
  const tray = "flex h-10 items-center rounded-md px-2";
  return (
    <>
      <Heading step={3} title="Find Viary next to the clock">
        Viary has no window on the taskbar. It waits in the system tray, ready whenever you hold {s.hotkeyName}.
      </Heading>
      <div aria-hidden="true" className="flex h-12 items-center justify-end gap-1 rounded-[10px] border border-line bg-[#EEF1F5] px-2">
        <span className={tray + " outline-2 -outline-offset-2 outline-[#0F5FB8]"}>
          <Icon name="chevronUp" size={16} />
        </span>
        <span className={tray + " bg-black/8"}>
          <Mark size={18} ink="#1B1B1B" voice="#1B1B1B" />
        </span>
        <span className={tray + " text-xs"}>14:02</span>
      </div>
      <ul className="m-0 flex list-none flex-col gap-3 p-0 text-sm leading-normal">
        <li className="flex gap-3">
          <Icon name="check" size={18} className="mt-0.5 shrink-0 text-teal" />
          <span>
            <b className="font-semibold">Windows hides new icons</b> under <span className="font-mono">^</span>. Drag Viary out
            of it onto the taskbar to keep it in view.
          </span>
        </li>
        <li className="flex gap-3">
          <Icon name="check" size={18} className="mt-0.5 shrink-0 text-teal" />
          <span>
            <b className="font-semibold">Click it</b> for status, language, and recent dictations.
          </span>
        </li>
        <li className="flex gap-3">
          <Icon name="check" size={18} className="mt-0.5 shrink-0 text-teal" />
          <span>
            <b className="font-semibold">Right-click it</b> to pause dictation, paste the last one again, or quit.
          </span>
        </li>
      </ul>
    </>
  );
}

function TryStep({ s }: { s: Snapshot }) {
  const field = useRef<HTMLTextAreaElement>(null);
  useEffect(() => field.current?.focus(), []);
  return (
    <>
      <Heading step={4} title="Say something">
        Hold {s.hotkeyName}, speak, and let go. Double-tap it to keep listening hands-free until you tap again.
      </Heading>
      <label className="flex flex-col gap-2">
        <span className="text-[13px] text-faint">Try it here</span>
        <textarea
          ref={field}
          rows={5}
          placeholder={`Hold ${s.hotkeyName}, say something, and let go.`}
          className="resize-none rounded-[10px] border border-rule bg-white px-3.5 py-3 text-[15px] leading-[1.6] outline-none focus:border-edge"
        />
      </label>
      <span className="text-[13px] text-muted">It works the same in any app: Mail, Word, your browser, a terminal.</span>
    </>
  );
}

/** GNOME's key caps, flat and grey. */
function Keys({ keys }: { keys: string[] }) {
  return (
    <span className="inline-flex items-center gap-1">
      {keys.map((k) => (
        <span key={k} className="inline-flex h-[26px] items-center rounded-md bg-[#EBEBEB] px-2 font-mono text-[13px]">
          {k}
        </span>
      ))}
    </span>
  );
}

/** What the extension's state means for the user. */
const EXTENSION: Record<Desktop["extension"], string> = {
  missing: "Not installed yet.",
  installed: "Installed. Log out and back in, and GNOME starts it.",
  active: "Running.",
  updated: "Running. Viary brought a newer version, which starts the next time you log in.",
};

/** The talk shortcut. On Wayland only Viary's GNOME Shell extension can
 *  hear it and type: installed here, it runs from the next login. */
function ShortcutStep({ s, desktop }: { s: Snapshot; desktop: Desktop }) {
  const [error, setError] = useState("");
  const [installing, setInstalling] = useState(false);
  const install = async () => {
    setError("");
    setInstalling(true);
    try {
      await api.installExtension();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setInstalling(false);
    }
  };
  const wayland = desktop.session === "wayland";
  const running = desktop.extension === "active" || desktop.extension === "updated";
  const talk = s.hotkeyName.split("+");
  // The extension's row says what it lacks; below, only other trouble.
  const trouble = !wayland || running ? shortcutTrouble(s) : null;
  return (
    <>
      <Heading step={1} title="Set up your talk shortcut">
        {wayland
          ? "On Wayland, apps can’t hear global keys or type into other windows. Viary’s GNOME Shell extension does both for it, and draws the pill at the bottom of the screen."
          : "On X11 Viary listens for the shortcut and types by itself; nothing to install."}
      </Heading>
      <ErrorBanner error={error} onDismiss={() => setError("")} />
      {wayland && (
        <Status
          ok={running}
          title="Viary’s GNOME Shell extension"
          text={EXTENSION[desktop.extension]}
          action={
            desktop.extension === "missing" && (
              <button type="button" className={btn} disabled={installing} onClick={install}>
                Install…
              </button>
            )
          }
        />
      )}
      <div className={boxed}>
        <div className={row}>
          <div className="flex grow flex-col">
            <span>Talk</span>
            <span className={sub}>Hold to speak, release to type</span>
          </div>
          <Keys keys={talk} />
        </div>
        <div className={row}>
          <div className="flex grow flex-col">
            <span>Hands-free</span>
            <span className={sub}>Tap twice to start, once to stop</span>
          </div>
          <Keys keys={talk} />
          <span className={sub}>×2</span>
        </div>
        <div className={row + " opacity-55"}>
          <div className="flex grow flex-col">
            <span>Edit selection</span>
            <span className={sub}>Hold over selected text · coming soon</span>
          </div>
          <Keys keys={["Ctrl", "Alt", "Shift", "Space"]} />
        </div>
      </div>
      {s.hotkeyActive ? <KeyTest name={s.hotkeyName} /> : trouble && <span className={sub}>{trouble}</span>}
    </>
  );
}

type Body = (props: { s: Snapshot; desktop: Desktop }) => React.ReactNode;

const BODIES: Body[] = GNOME
  ? [GnomeMicrophoneStep, ShortcutStep, GnomeEngineStep, GnomeTryStep]
  : [MicrophoneStep, TalkKeyStep, EngineStep, TrayStep, TryStep];

/** The steps that may be skipped: the voice engine, and on GNOME the talk
 *  shortcut, which waits for a login when the extension is new. */
const ENGINE = 2;
const SHORTCUT = GNOME ? 1 : -1;

function clock(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** A dictation's progress in words, from what the pill shows. */
function progress(view: PillView, now: number): { text: string; tone: "live" | "busy" | "done" | "warn" } | null {
  switch (view.kind) {
    case "listening":
    case "handsFree":
      return { text: `Listening · ${clock(now - view.startedAt)}`, tone: "live" };
    case "transcribing":
      return { text: view.label, tone: "busy" };
    case "polishing":
      return { text: "Polishing…", tone: "busy" };
    case "inserted":
      return { text: view.label, tone: "done" };
    case "copied":
      return { text: `${view.label} · ${view.hint}`, tone: "done" };
    case "failed":
      return { text: view.message, tone: "warn" };
    case "hint":
      return { text: view.text, tone: "warn" };
    default:
      return null;
  }
}

const TONE = {
  live: "bg-[#FF6B3D]",
  busy: "bg-[#3584E4]",
  done: "bg-[#26734D]",
  warn: "bg-[#E5A50A]",
};

/** A first dictation on GNOME, with its progress shown here as well as in
 *  the pill. */
function GnomeTryStep({ s }: { s: Snapshot }) {
  const field = useRef<HTMLTextAreaElement>(null);
  useEffect(() => field.current?.focus(), []);
  const view = usePillView();
  const [now, setNow] = useState(Date.now());
  const [worked, setWorked] = useState(false);
  const listening = view.kind === "listening" || view.kind === "handsFree";
  useEffect(() => {
    if (!listening) return;
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [listening]);
  useEffect(() => {
    if (view.kind === "inserted" || view.kind === "copied") setWorked(true);
  }, [view.kind]);
  const key = s.hotkeyName;
  const state = progress(view, now);
  return (
    <>
      <Heading step={3} title="Say something">
        {`Hold ${key}, speak, and let go. Double-tap it to keep listening hands-free until you tap again.`}
      </Heading>
      <div className={boxed}>
        <textarea
          ref={field}
          rows={5}
          aria-label="Try it here"
          placeholder={`Hold ${key} and say something.`}
          className="block w-full resize-none bg-white px-4 py-3 text-[15px] leading-[1.6] outline-none"
        />
        <div role="status" className="flex min-h-11 items-center gap-2.5 border-t border-black/8 px-4 text-[13px]">
          {state ? (
            <>
              <span aria-hidden="true" className={"size-2 shrink-0 rounded-full " + TONE[state.tone]} />
              <span className="truncate">{state.text}</span>
            </>
          ) : worked ? (
            <span className="flex items-center gap-2 text-[#26734D]">
              <Icon name="check" size={16} strokeWidth={2.2} />
              It works. From now on Viary waits in the top bar.
            </span>
          ) : (
            <span className="text-[#5E5E5E]">Waiting for {key}…</span>
          )}
        </div>
      </div>
      <span className={sub}>It works the same in any app: Text Editor, Firefox, Terminal, LibreOffice.</span>
    </>
  );
}

/** Whether a step's requirement is met, so Continue can go on. */
function ready(step: number, s: Snapshot): boolean {
  if (step === 0) return s.permissions.microphone !== false;
  if (step === SHORTCUT) return s.hotkeyActive;
  if (step === ENGINE) return !!s.engine.active;
  return true;
}

export function Setup() {
  const [s] = useSnapshot();
  const [step, setStep] = useState(0);
  if (!s) return null;
  // Windows has no desktop; its steps never read this.
  const desktop: Desktop = s.desktop ?? { session: "x11", extension: "missing" };
  const last = step === STEPS.length - 1;
  const Body = BODIES[step];
  const footer = (
    <div className="flex justify-between">
      <button type="button" className={btn} disabled={step === 0} onClick={() => setStep(step - 1)}>
        Back
      </button>
      <div className="flex gap-2">
        {(step === ENGINE || step === SHORTCUT) && !ready(step, s) && (
          <button type="button" className={btn} onClick={() => setStep(step + 1)}>
            Skip for now
          </button>
        )}
        <button
          type="button"
          className={btnPrimary}
          disabled={!ready(step, s)}
          onClick={() => (last ? api.finishSetup() : setStep(step + 1))}
        >
          {last ? "Done" : "Continue"}
        </button>
      </div>
    </div>
  );

  if (GNOME) {
    return (
      <div className="flex h-full flex-col overflow-hidden bg-[#FAFAFA] text-[#1E1E1E]">
        <HeaderBar />
        <div className="flex min-h-0 grow">
          <aside className="w-[260px] shrink-0 border-r border-[#DADADA] bg-[#F0F0F0] px-4 py-6">
            <GnomeSteps current={step} />
          </aside>
          <main className="scroll flex grow flex-col gap-5 px-11 py-9">
            <Body s={s} desktop={desktop} />
            <div className="grow" />
            {footer}
          </main>
        </div>
      </div>
    );
  }
  return (
    <div className="flex h-full flex-col overflow-hidden bg-paper">
      <TitleBar />
      <div className="flex min-h-0 grow">
        <aside className="flex w-[280px] shrink-0 flex-col gap-[26px] border-r border-rule bg-sand px-7 py-8">
          <span className="flex items-center gap-2.5">
            <Mark size={32} ink="#1C1B18" voice="#E0521F" />
            <span className="-mt-1 font-serif text-[28px] leading-none font-medium tracking-[-0.02em]">viary</span>
          </span>
          <Steps current={step} />
          <div className="grow" />
          <span className="text-xs leading-normal text-faint">{NOTES[step]}</span>
        </aside>
        <main className="scroll flex grow flex-col gap-[22px] px-12 pt-11 pb-8">
          <Body s={s} desktop={desktop} />
          <div className="grow" />
          {footer}
        </main>
      </div>
    </div>
  );
}
