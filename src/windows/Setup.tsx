// The setup window on Windows (WinOnboarding artboard) and Ubuntu
// (LinuxShortcut, LinuxTyping): five steps, from the microphone to a first
// dictation. Viary runs in the tray all along; closing the window leaves
// setup for the next launch.

import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useCallback, useEffect, useRef, useState } from "react";
import { ErrorBanner, Switch } from "../components/Controls";
import { Icon, Mark } from "../components/Icons";
import { AddModel } from "../pages/Engine";
import { api, errorText, useHotkey, useSnapshot, type Desktop, type Hotkey, type Snapshot, type Typing } from "../lib/ipc";
import { PLATFORM, THIS_COMPUTER } from "../lib/platform";

/** GNOME's look and steps: Ubuntu. Windows otherwise. */
const GNOME = PLATFORM === "linux";

const STEPS = GNOME
  ? ["Microphone", "Talk shortcut", "Typing into apps", "Voice engine", "Try it"]
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
      <button type="button" aria-label="Close" className={caption + " hover:bg-[#C42B1C] hover:text-white"} onClick={() => window.hide()}>
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
        onClick={() => getCurrentWebviewWindow().hide()}
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
      {GNOME ? (
        <span className={sub}>GNOME Settings › Privacy › Microphone turns the microphone off for every app at once.</span>
      ) : (
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
      )}
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

const KEYS: { key: Hotkey; caps: string[]; label: string; note: string }[] = [
  { key: "rightAlt", caps: ["Right Alt"], label: "Right Alt", note: "Recommended" },
  { key: "ctrlWin", caps: ["Ctrl", "Win"], label: "Ctrl + Win", note: "Left side, two keys" },
];

/** Press the talk key to see Viary hear it. While shown, the key only
 *  reports itself: no dictation, no hints. */
function KeyTest({ name, toggle = false }: { name: string; toggle?: boolean }) {
  useEffect(() => {
    api.setKeyTest(true);
    return () => {
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
  const text = tested
    ? toggle
      ? `Detected: ${name}`
      : `Detected: ${name} held for ${(held / 1000).toFixed(1)} s`
    : toggle
      ? `Press ${name} once.`
      : `Hold ${name} for a moment, then release it.`;
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
              <small className="text-xs font-normal text-faint">{k.note}</small>
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
      <div className="rounded-[10px] border border-[#F0DDB6] bg-[#FFF6E6] px-4 py-3 text-[13px] leading-[1.45] text-[#6B4A00]">
        On keyboards where Right Alt is AltGr (German, French, Polish…), choose Ctrl + Win instead so accented letters keep
        working. Win + H stays with Windows voice typing.
      </div>
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

const TALK = ["Ctrl", "Alt", "Space"];

function ShortcutStep({ s, desktop }: { s: Snapshot; desktop: Desktop }) {
  const { mode, bound } = desktop.shortcut;
  const toggle = mode === "toggle";
  const x11 = desktop.session === "x11";
  return (
    <>
      <Heading step={1} title="Set your talk shortcut">
        {x11
          ? "On X11 Viary listens for the shortcut itself; nothing to allow."
          : "GNOME keeps global shortcuts for itself. Viary asks it to hand these over; you can change them later in GNOME Settings."}
      </Heading>
      <div className={boxed}>
        <div className={row}>
          <div className="flex grow flex-col">
            <span>Talk</span>
            <span className={sub}>{toggle ? "Press to start, press again to type" : "Hold to speak, release to type"}</span>
          </div>
          <Keys keys={TALK} />
        </div>
        {!toggle && (
          <div className={row}>
            <div className="flex grow flex-col">
              <span>Hands-free</span>
              <span className={sub}>Tap twice to start, once to stop</span>
            </div>
            <Keys keys={TALK} />
            <span className={sub}>×2</span>
          </div>
        )}
        <div className={row + " opacity-55"}>
          <div className="flex grow flex-col">
            <span>Edit selection</span>
            <span className={sub}>Hold over selected text · coming soon</span>
          </div>
          <Keys keys={["Ctrl", "Alt", "Shift", "Space"]} />
        </div>
      </div>
      {toggle && (
        <div className="rounded-xl bg-[#FDF3D6] px-4 py-3 text-[13px] leading-normal text-[#5C4400]">
          This GNOME can’t tell Viary when a key is released. Here, Talk is a toggle: press to start, press again to
          type. Viary adds it as a GNOME custom shortcut.
        </div>
      )}
      {bound && <KeyTest name={s.hotkeyName} toggle={toggle} />}
    </>
  );
}

const TYPING: { id: Typing; title: string; text: string }[] = [
  {
    id: "extension",
    title: "Viary’s GNOME Shell extension",
    text: "Types directly and adds the floating pill at the bottom of the screen. No prompts, works in every app.",
  },
  {
    id: "portal",
    title: "Ask GNOME for keyboard access",
    text: "Uses GNOME’s remote-interaction permission. GNOME asks once, and shows a sharing icon in the top bar while Viary types.",
  },
  {
    id: "clipboard",
    title: "Clipboard only",
    text: "Viary copies the text and tells you; you press Ctrl+V. Nothing extra to allow.",
  },
];

function TypingStep({ desktop }: { s: Snapshot; desktop: Desktop }) {
  const [error, setError] = useState("");
  const [asking, setAsking] = useState(false);
  const run = async (work: () => Promise<void>) => {
    setError("");
    setAsking(true);
    try {
      await work();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setAsking(false);
    }
  };
  const choose = (method: Typing) => run(() => api.setTyping(method));
  const install = () => run(() => api.installExtension());
  if (desktop.session === "x11") {
    return (
      <>
        <Heading step={2} title="Typing into apps">
          You are running an X11 session: Viary types directly, and none of the Wayland choices apply.
        </Heading>
        <Status ok title="Viary can type into other apps" text="Text goes in with Ctrl+V, and your clipboard is put back." />
      </>
    );
  }
  return (
    <>
      <Heading step={2} title="How should Viary type?">
        On Wayland, apps can’t type into other windows on their own. Pick how GNOME lets Viary do it.
      </Heading>
      <ErrorBanner error={error} onDismiss={() => setError("")} />
      <div role="radiogroup" aria-label="Typing method" className={boxed}>
        {TYPING.map((t) => {
          // The extension can be chosen once it runs: from the next login.
          const waiting = t.id === "extension" && desktop.extension !== "active";
          const on = !waiting && desktop.typing === t.id;
          return (
            <div key={t.id} className={row + " py-3.5"}>
              <button
                type="button"
                role="radio"
                aria-checked={on}
                disabled={waiting || asking}
                onClick={() => choose(t.id)}
                className="flex grow items-center gap-3 text-left disabled:cursor-default"
              >
                <span
                  aria-hidden="true"
                  className={
                    "size-[18px] shrink-0 rounded-full " +
                    (on ? "border-[5px] border-[#C34113]" : "border-[1.5px] border-[#B0B0B0]") +
                    (waiting ? " opacity-50" : "")
                  }
                />
                <span className="flex grow flex-col gap-1">
                  <span className="flex items-center gap-2">
                    {t.title}
                    {t.id === "extension" && (
                      <span className="rounded-full bg-[#E1F2E8] px-2 py-0.5 text-[11px] font-semibold text-[#1D5E3C]">Recommended</span>
                    )}
                  </span>
                  <span className={sub}>
                    {t.id === "extension" && desktop.extension === "installed"
                      ? "Installed. Log out and back in, and GNOME starts it; then choose it here."
                      : t.text}
                  </span>
                </span>
              </button>
              {t.id === "extension" && desktop.extension === "missing" && (
                <button type="button" className={btn} disabled={asking} onClick={install}>
                  Install…
                </button>
              )}
            </div>
          );
        })}
      </div>
      {desktop.typing === "portal" && desktop.portalAllowed && (
        <span className={sub}>GNOME allowed Viary to type. You can take it back in GNOME Settings › Privacy.</span>
      )}
    </>
  );
}

type Body = (props: { s: Snapshot; desktop: Desktop }) => React.ReactNode;

const BODIES: Body[] = GNOME
  ? [MicrophoneStep, ShortcutStep, TypingStep, EngineStep, TryStep]
  : [MicrophoneStep, TalkKeyStep, EngineStep, TrayStep, TryStep];

/** The voice engine step, which may be skipped. */
const ENGINE = GNOME ? 3 : 2;

/** Whether a step's requirement is met, so Continue can go on. */
function ready(step: number, s: Snapshot): boolean {
  if (step === 0) return s.permissions.microphone !== false;
  if (step === ENGINE) return !!s.engine.active;
  return true;
}

/** The talk shortcut step on GNOME: until GNOME hands it over, the main
 *  button asks for it. */
function asksGnome(step: number, desktop: Desktop | null): boolean {
  return GNOME && step === 1 && !!desktop && !desktop.shortcut.bound;
}

export function Setup() {
  const [s] = useSnapshot();
  const [step, setStep] = useState(0);
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState("");
  if (!s) return null;
  // Windows has no desktop choices; its steps never read these.
  const desktop: Desktop = s.desktop ?? {
    session: "x11",
    shortcut: { mode: "hold", bound: true },
    typing: "clipboard",
    portalAllowed: false,
    extension: "missing",
  };
  const last = step === STEPS.length - 1;
  const Body = BODIES[step];
  const ask = async () => {
    setError("");
    setAsking(true);
    try {
      await api.bindShortcut();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setAsking(false);
    }
  };
  const footer = (
    <div className="flex justify-between">
      <button type="button" className={btn} disabled={step === 0} onClick={() => setStep(step - 1)}>
        Back
      </button>
      <div className="flex gap-2">
        {step === ENGINE && !ready(step, s) && (
          <button type="button" className={btn} onClick={() => setStep(step + 1)}>
            Skip for now
          </button>
        )}
        {asksGnome(step, s.desktop) ? (
          <button type="button" className={btnPrimary} disabled={asking} onClick={ask}>
            {desktop.shortcut.mode === "toggle" ? "Add the shortcut" : "Ask GNOME…"}
          </button>
        ) : (
          <button
            type="button"
            className={btnPrimary}
            disabled={!ready(step, s)}
            onClick={() => (last ? api.finishSetup() : setStep(step + 1))}
          >
            {last ? "Done" : "Continue"}
          </button>
        )}
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
            <ErrorBanner error={error} onDismiss={() => setError("")} />
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
