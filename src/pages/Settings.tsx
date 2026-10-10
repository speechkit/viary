// Settings: the dictation key and the permissions Viary needs, and on
// Linux the talk shortcut GNOME hands over and how Viary types.

import { useState } from "react";
import { ErrorBanner, Switch } from "../components/Controls";
import { Icon, Kbd } from "../components/Icons";
import { api, errorText, shortcutTrouble, type Desktop, type Hotkey, type Snapshot } from "../lib/ipc";
import { PLATFORM, type Platform } from "../lib/platform";
import { KeyTest } from "../windows/Setup";

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
        {KEYS[PLATFORM].map(([key, label, plain]) => {
          const on = s.settings.hotkey === key;
          // On an AltGr layout, Right Alt types characters: Ctrl + Win it is.
          const text = !s.keyboard?.altGr
            ? plain
            : key === "rightAlt"
              ? "The Alt key right of the space bar. It types é, @… on your keyboard layout."
              : key === "ctrlWin"
                ? "Both keys left of the space bar, held together. Recommended for your keyboard."
                : plain;
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
      {PLATFORM === "windows" && <WindowsKey s={s} />}

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

/** Linux: the talk shortcut, Viary's GNOME Shell extension, which hears
 *  it and types on Wayland, and the microphone. The same state as setup. */
function LinuxSettings({ s, desktop }: { s: Snapshot; desktop: Desktop }) {
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const install = async () => {
    setError("");
    setBusy(true);
    try {
      await api.installExtension();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const wayland = desktop.session === "wayland";
  const extension = desktop.extension;
  return (
    <>
      <ErrorBanner error={error} onDismiss={() => setError("")} />

      <h2 className={h2}>Talk shortcut</h2>
      <div className={card}>
        <Row
          tone={s.hotkeyActive ? "ok" : "attention"}
          title={s.hotkeyName}
          text={
            shortcutTrouble(s) ??
            (wayland
              ? "Viary’s extension hears it: hold to speak, release to insert, double-tap for hands-free."
              : "Viary listens for it itself on X11.")
          }
        />
      </div>

      <h2 className={h2}>Typing into apps</h2>
      <div className={card}>
        {wayland ? (
          <Row
            tone={extension === "active" || extension === "updated" ? "ok" : "attention"}
            title="Viary’s GNOME Shell extension"
            text={
              extension === "missing"
                ? "Hears the talk shortcut, types directly, and draws the pill at the bottom of the screen. Not installed."
                : extension === "installed"
                  ? "Installed. GNOME starts it the next time you log in."
                  : extension === "updated"
                    ? "Running. Updated with Viary: the new version runs the next time you log in."
                    : "Running: it types directly and draws the pill at the bottom of the screen."
            }
          >
            {extension === "missing" && (
              <button type="button" className={btn} disabled={busy} onClick={install}>
                Install…
              </button>
            )}
          </Row>
        ) : (
          <Row tone="ok" title="Viary types directly" text="On X11, text goes in with Ctrl+V, and your clipboard is put back." />
        )}
      </div>

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

const note = "rounded-[10px] border border-[#F0DDB6] bg-[#FFF6E6] px-4 py-3 text-[13px] leading-[1.45] text-[#6B4A00]";

/** Windows: whether the layout makes Right Alt a poor talk key, whether
 *  the keyboard hook runs, and a test of the key. */
function WindowsKey({ s }: { s: Snapshot }) {
  const [testing, setTesting] = useState(false);
  const altGr = s.keyboard?.altGr ?? false;
  return (
    <>
      {altGr && s.settings.hotkey === "rightAlt" && (
        <div className={note + " flex items-center gap-3"}>
          <span className="grow">
            Your keyboard layout uses Right Alt as AltGr for characters like é and @. Holding it to talk gets in the way of
            typing them; Ctrl + Win does not.
          </span>
          <button type="button" className={btn} onClick={() => api.setPreferences({ hotkey: "ctrlWin" })}>
            Use Ctrl + Win
          </button>
        </div>
      )}
      {!s.hotkeyActive && (
        <div className={note}>
          Viary can’t see the keyboard right now, so {s.hotkeyName} does nothing. Quit Viary from the tray and open it
          again.
        </div>
      )}
      {testing ? (
        <div className="flex flex-col gap-2">
          <KeyTest key={s.hotkeyName} name={s.hotkeyName} />
          <span className="flex items-center gap-3 text-[13px] text-muted">
            <span className="grow">While this test is open, {s.hotkeyName} starts no dictation.</span>
            <button type="button" className={btn} onClick={() => setTesting(false)}>
              Done
            </button>
          </span>
        </div>
      ) : (
        s.hotkeyActive && (
          <span>
            <button type="button" className={btn} onClick={() => setTesting(true)}>
              Test {s.hotkeyName}
            </button>
          </span>
        )
      )}
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

      <h2 className={h2}>Tray icon</h2>
      <div className={card}>
        <Row
          title="Keep Viary in view"
          text="Windows hides new tray icons under ^. Turn Viary on under Taskbar settings › Other system tray icons, or drag it onto the taskbar."
        >
          <button type="button" className={btn} onClick={() => api.requestPermission("taskbar")}>
            Taskbar Settings
          </button>
        </Row>
      </div>
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
