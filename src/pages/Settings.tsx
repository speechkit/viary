// Settings: the dictation key and the permissions Viary needs, and on
// Linux the talk shortcut GNOME hands over and how Viary types.

import { useState } from "react";
import { ErrorBanner, Switch } from "../components/Controls";
import { Icon, Kbd } from "../components/Icons";
import { api, errorText, type Desktop, type Hotkey, type Snapshot, type Typing } from "../lib/ipc";
import { PLATFORM, type Platform } from "../lib/platform";

const btn = "inline-flex h-8 shrink-0 items-center whitespace-nowrap gap-2 rounded-lg border border-edge bg-white px-3 text-[13px] font-medium text-ink";

const KEYS: Record<Platform, [Hotkey, string, string][]> = {
  macos: [
    ["fn", "fn", "The globe key. Set System Settings › Keyboard › “Press 🌐 key to” to “Do Nothing”."],
    ["rightOption", "right ⌥", "The Option key right of the space bar."],
    ["rightCommand", "right ⌘", "The Command key right of the space bar."],
  ],
  windows: [
    ["rightAlt", "Right Alt", "The Alt key right of the space bar. Recommended."],
    ["ctrlWin", "Ctrl + Win", "Both keys left of the space bar, held together."],
  ],
  // GNOME's shortcut is not a choice here: see LinuxSettings.
  linux: [],
};

const h2 = "m-0 text-xs font-semibold tracking-[.04em] text-faint uppercase";
const card = "overflow-hidden rounded-[14px] border border-line bg-white";

/** A settings row: a status dot, a title and its text, and an action. */
function Row({
  tone,
  title,
  text,
  children,
}: {
  tone?: "ok" | "attention";
  title: string;
  text: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-3.5 border-b border-hair px-[18px] py-3 last:border-b-0">
      {tone && <span className={`size-2 shrink-0 rounded-full ${tone === "ok" ? "bg-teal" : "bg-amber"}`} />}
      <div className="flex min-w-0 grow flex-col gap-0.5">
        <span className="text-sm font-medium">{title}</span>
        <span className="text-[13px] leading-[1.45] text-muted">{text}</span>
      </div>
      {children}
    </div>
  );
}

function Permission({
  granted,
  title,
  text,
  kind,
}: {
  granted: boolean;
  title: string;
  text: string;
  kind: "accessibility" | "inputMonitoring" | "microphone";
}) {
  return (
    <div className="flex items-center gap-3.5 border-b border-hair px-[18px] py-3 last:border-b-0">
      <span className={`size-2 shrink-0 rounded-full ${granted ? "bg-teal" : "bg-amber"}`} />
      <div className="flex min-w-0 grow flex-col gap-0.5">
        <span className="text-sm font-medium">{title}</span>
        <span className="text-[13px] leading-[1.45] text-muted">{text}</span>
      </div>
      {granted ? (
        <span className="text-[13px] text-teal-ink">Allowed</span>
      ) : (
        <button type="button" className={btn} onClick={() => api.requestPermission(kind)}>
          Open System Settings
        </button>
      )}
    </div>
  );
}

export function SettingsPage({ s }: { s: Snapshot }) {
  if (PLATFORM === "linux" && s.desktop) {
    return (
      <main className="flex max-w-[900px] flex-col gap-[18px] px-12 pt-9 pb-12">
        <h1 className="m-0 font-serif text-[40px] font-normal">Settings</h1>
        <LinuxSettings s={s} desktop={s.desktop} />
        <Startup s={s} />
      </main>
    );
  }
  return (
    <main className="flex max-w-[900px] flex-col gap-[18px] px-12 pt-9 pb-12">
      <h1 className="m-0 font-serif text-[40px] font-normal">Settings</h1>

      <h2 className="m-0 text-xs font-semibold tracking-[.04em] text-faint uppercase">Dictation key</h2>
      <div role="radiogroup" aria-label="Dictation key" className="grid grid-cols-3 gap-3.5">
        {KEYS[PLATFORM].map(([key, label, text]) => {
          const on = s.settings.hotkey === key;
          return (
            <button
              key={key}
              type="button"
              role="radio"
              aria-checked={on}
              onClick={() => api.setPreferences({ hotkey: key })}
              className={
                "flex flex-col gap-2.5 rounded-[14px] bg-white px-[18px] py-4 text-left " +
                (on ? "border-2 border-ink" : "border border-line")
              }
            >
              <span className="flex w-full items-center justify-between">
                <Kbd large>{label}</Kbd>
                <span className="size-[18px] rounded-full" style={{ border: on ? "5px solid #1C1B18" : "1.5px solid #CFC8B8" }} />
              </span>
              <span className="text-[13px] leading-[1.45] text-muted">{text}</span>
            </button>
          );
        })}
      </div>
      <span className="text-[13px] text-muted">Hold the key while you speak and release it to insert. Double-tap it for hands-free: Viary listens until you tap it again.</span>
      {PLATFORM === "windows" && (
        <span className="rounded-[10px] border border-[#F0DDB6] bg-[#FFF6E6] px-4 py-3 text-[13px] leading-[1.45] text-[#6B4A00]">
          On keyboards where Right Alt is AltGr (German, French, Polish…), choose Ctrl + Win so accented letters keep
          working. Win + H stays with Windows voice typing.
        </span>
      )}

      {PLATFORM === "windows" ? <WindowsPermissions s={s} /> : <MacPermissions s={s} />}
      <Startup s={s} />
    </main>
  );
}

/** Starting with the system, which setup offers on Windows. */
function Startup({ s }: { s: Snapshot }) {
  const when = { macos: "you log in to your Mac", windows: "you sign in to Windows", linux: "you log in" }[PLATFORM];
  return (
    <>
      <h2 className={h2}>Startup</h2>
      <div className={card}>
        <Row title="Start at login" text={`Viary starts in the ${PLATFORM === "macos" ? "menu bar" : PLATFORM === "windows" ? "tray" : "top bar"} when ${when}.`}>
          <Switch on={s.autostart} label="Start Viary at login" onChange={(on) => api.setAutostart(on)} />
        </Row>
      </div>
    </>
  );
}

const TYPING: { id: Typing; title: string; text: string }[] = [
  { id: "extension", title: "Viary’s GNOME Shell extension", text: "Types directly, no prompts, and draws the pill at the bottom of the screen." },
  { id: "portal", title: "GNOME keyboard access", text: "GNOME asks once, and shows a sharing icon in the top bar while Viary types." },
  { id: "clipboard", title: "Clipboard only", text: "Viary copies the text and tells you; you press Ctrl+V." },
];

/** Linux: the talk shortcut GNOME hands over, how Viary types, its GNOME
 *  Shell extension, and the microphone. The same state as setup. */
function LinuxSettings({ s, desktop }: { s: Snapshot; desktop: Desktop }) {
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const run = async (work: () => Promise<void>) => {
    setError("");
    setBusy(true);
    try {
      await work();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const { mode, bound } = desktop.shortcut;
  const toggle = mode === "toggle";
  const wayland = desktop.session === "wayland";
  const extension = desktop.extension;
  return (
    <>
      <ErrorBanner error={error} onDismiss={() => setError("")} />

      <h2 className={h2}>Talk shortcut</h2>
      <div className={card}>
        <Row
          tone={bound ? "ok" : "attention"}
          title={s.hotkeyName}
          text={
            !wayland
              ? "Viary listens for it itself on X11."
              : bound
                ? toggle
                  ? "A GNOME custom shortcut: press to start, press again to insert. This GNOME can’t report the key being released."
                  : "GNOME hands it to Viary: hold to speak, release to insert, double-tap for hands-free."
                : "GNOME has not handed it to Viary yet."
          }
        >
          {wayland && !bound ? (
            <button type="button" className={btn} disabled={busy} onClick={() => run(api.bindShortcut)}>
              {toggle ? "Add the shortcut" : "Ask GNOME…"}
            </button>
          ) : (
            wayland && (
              <button type="button" className={btn} onClick={() => api.requestPermission("inputMonitoring")}>
                Keyboard Settings
              </button>
            )
          )}
        </Row>
      </div>
      {wayland && bound && (
        <span className="text-[13px] text-muted">
          To use other keys, change Viary’s shortcut in GNOME Settings › Keyboard.
        </span>
      )}

      <h2 className={h2}>Typing into apps</h2>
      {wayland ? (
        <div role="radiogroup" aria-label="Typing method" className={card}>
          {TYPING.map((t) => {
            const waiting = t.id === "extension" && extension !== "active";
            const on = !waiting && desktop.typing === t.id;
            return (
              <div key={t.id} className="flex items-center gap-3.5 border-b border-hair px-[18px] py-3 last:border-b-0">
                <button
                  type="button"
                  role="radio"
                  aria-checked={on}
                  disabled={waiting || busy}
                  onClick={() => run(() => api.setTyping(t.id))}
                  className="flex min-w-0 grow items-center gap-3.5 text-left disabled:cursor-default"
                >
                  <span
                    aria-hidden="true"
                    className={"size-[18px] shrink-0 rounded-full" + (waiting ? " opacity-50" : "")}
                    style={{ border: on ? "5px solid #1C1B18" : "1.5px solid #CFC8B8" }}
                  />
                  <span className="flex min-w-0 grow flex-col gap-0.5">
                    <span className="text-sm font-medium">{t.title}</span>
                    <span className="text-[13px] leading-[1.45] text-muted">
                      {t.id === "extension" && extension === "missing"
                        ? `${t.text} Not installed.`
                        : t.id === "extension" && extension === "installed"
                          ? "Installed. GNOME starts it the next time you log in."
                          : t.id === "portal" && desktop.portalAllowed
                            ? "GNOME allowed it. Take it back in GNOME Settings › Privacy."
                            : t.text}
                    </span>
                  </span>
                </button>
                {t.id === "extension" && extension === "missing" && (
                  <button type="button" className={btn} disabled={busy} onClick={() => run(api.installExtension)}>
                    Install…
                  </button>
                )}
              </div>
            );
          })}
        </div>
      ) : (
        <div className={card}>
          <Row tone="ok" title="Viary types directly" text="On X11, text goes in with Ctrl+V, and your clipboard is put back." />
        </div>
      )}

      <h2 className={h2}>Microphone</h2>
      <div className={card}>
        <Row title="Input device and volume" text="Viary uses the microphone chosen under Voice engine, at the volume GNOME sets.">
          <button type="button" className={btn} onClick={() => api.requestPermission("microphone")}>
            Sound Settings
          </button>
        </Row>
      </div>
    </>
  );
}

/** Windows asks for nothing but the microphone switch. */
function WindowsPermissions({ s }: { s: Snapshot }) {
  return (
    <>
      <h2 className="m-0 text-xs font-semibold tracking-[.04em] text-faint uppercase">Permissions</h2>
      <div className="overflow-hidden rounded-[14px] border border-line bg-white">
        <div className="flex items-center gap-3.5 px-[18px] py-3">
          <span className={`size-2 shrink-0 rounded-full ${s.permissions.microphone !== false ? "bg-teal" : "bg-amber"}`} />
          <div className="flex min-w-0 grow flex-col gap-0.5">
            <span className="text-sm font-medium">Microphone</span>
            <span className="text-[13px] leading-[1.45] text-muted">
              “Let desktop apps access your microphone” in Settings › Privacy &amp; security › Microphone.
            </span>
          </div>
          {s.permissions.microphone !== false ? (
            <span className="text-[13px] text-teal-ink">Allowed</span>
          ) : (
            <button type="button" className={btn} onClick={() => api.requestPermission("microphone")}>
              Open Settings
            </button>
          )}
        </div>
      </div>
      <span className="text-[13px] text-muted">
        Viary cannot type into apps running as administrator; there, the text stays on the clipboard.
      </span>
    </>
  );
}

function MacPermissions({ s }: { s: Snapshot }) {
  return (
    <>

      <h2 className="m-0 text-xs font-semibold tracking-[.04em] text-faint uppercase">Permissions</h2>
      <div className="overflow-hidden rounded-[14px] border border-line bg-white">
        <Permission
          granted={s.permissions.inputMonitoring && s.hotkeyActive}
          kind="inputMonitoring"
          title="Input Monitoring"
          text={`To notice when you hold ${s.hotkeyName}. Viary does not record other keys.`}
        />
        <Permission
          granted={s.permissions.accessibility}
          kind="accessibility"
          title="Accessibility"
          text="To paste into the app you are using, and to check that a text field has focus."
        />
        <div className="flex items-center gap-3.5 px-[18px] py-3">
          <Icon name="mic" />
          <div className="flex min-w-0 grow flex-col gap-0.5">
            <span className="text-sm font-medium">Microphone</span>
            <span className="text-[13px] leading-[1.45] text-muted">macOS asks the first time you dictate.</span>
          </div>
          <button type="button" className={btn} onClick={() => api.requestPermission("microphone")}>
            Open System Settings
          </button>
        </div>
      </div>
      <span className="text-[13px] text-muted">
        After allowing a permission, macOS may ask you to quit and reopen Viary.
      </span>
    </>
  );
}
