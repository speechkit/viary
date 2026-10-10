// The system Viary runs on, and the words and keys that differ by it: ⌘ on
// macOS is Ctrl on Windows and Linux, the Keychain is Credential Manager or
// the GNOME keyring, the menu bar is the system tray or the top bar.

export type Platform = "macos" | "windows" | "linux";

function detect(): Platform {
  // `?os=windows` on the preview page renders another system's windows.
  const forced = new URLSearchParams(window.location.search).get("os");
  if (forced === "macos" || forced === "windows" || forced === "linux") return forced;
  const agent = navigator.userAgent;
  if (agent.includes("Windows")) return "windows";
  if (agent.includes("Mac")) return "macos";
  return "linux";
}

export const PLATFORM: Platform = detect();
export const isMac = PLATFORM === "macos";

/** Whether the command key is held: ⌘ on macOS, Ctrl elsewhere. Not while
 *  AltGr is: Windows reports it as Ctrl+Alt, and AltGr+key types a
 *  character (é, @, [) rather than a shortcut. */
export function cmd(e: { metaKey: boolean; ctrlKey: boolean; getModifierState?: (key: string) => boolean }): boolean {
  if (isMac) return e.metaKey;
  return e.ctrlKey && !e.getModifierState?.("AltGraph");
}

type Modifier = "cmd" | "alt" | "shift";

/** A shortcut as the system writes it: "⌥⌘N" on macOS, "Ctrl+Alt+N" elsewhere. */
export function shortcut(modifiers: Modifier[], key: string): string {
  const has = (m: Modifier) => modifiers.includes(m);
  if (isMac) return (has("alt") ? "⌥" : "") + (has("shift") ? "⇧" : "") + (has("cmd") ? "⌘" : "") + key;
  return [has("cmd") && "Ctrl", has("alt") && "Alt", has("shift") && "Shift", key].filter(Boolean).join("+");
}

/** Where API keys are kept. */
export const KEY_STORE = { macos: "the Keychain", windows: "Windows Credential Manager", linux: "the GNOME keyring" }[PLATFORM];

/** Where Viary's icon lives. */
export const TRAY = { macos: "menu bar", windows: "system tray", linux: "top bar" }[PLATFORM];

/** This computer, as a model folder's location. */
export const THIS_COMPUTER = { macos: "this Mac", windows: "this PC", linux: "this computer" }[PLATFORM];

/** The system's settings app. */
export const SYSTEM_SETTINGS = { macos: "System Settings", windows: "Settings", linux: "Settings" }[PLATFORM];

/** New voice note, from any app. Windows reports AltGr as Ctrl+Alt, so
 *  there it is Win+Alt+N, as lib.rs registers it. */
export const NEW_NOTE = PLATFORM === "windows" ? "Win+Alt+N" : shortcut(["cmd", "alt"], "N");

/** Mark the moment in a voice note. */
export const MARK = shortcut(["cmd"], "M");
