// The first-launch tip on Windows (WinTray artboard, 03): Windows hides a
// new tray icon under ^, so after setup Viary says where it went, once.
// "Show me" opens the taskbar settings, where the icon can be kept in view.

import { Icon, Mark } from "../components/Icons";
import { api, useSnapshot } from "../lib/ipc";

const button = "h-8 rounded border px-3.5 text-sm";

export function TrayTip() {
  const [s] = useSnapshot();
  if (!s) return null;
  return (
    <div className="p-3">
      <div
        role="status"
        className="flex w-[364px] flex-col overflow-hidden rounded-lg bg-white font-[Segoe_UI_Variable_Text,Segoe_UI,var(--font-sans)] text-[#1B1B1B] shadow-[0_16px_40px_rgba(0,0,0,0.28),0_0_0_1px_rgba(0,0,0,0.1)]"
      >
        <div className="flex items-center gap-2 px-4 pt-3 text-xs text-[#616161]">
          <span aria-hidden="true" className="inline-flex size-4 items-center justify-center rounded bg-ink">
            <Mark size={10} />
          </span>
          Viary
          <span className="grow" />
          <button
            type="button"
            aria-label="Dismiss"
            onClick={() => api.closeTrayTip(false)}
            className="flex size-6 items-center justify-center rounded hover:bg-black/5"
          >
            <Icon name="close" size={12} />
          </button>
        </div>
        <div className="flex flex-col gap-1.5 px-4 pt-2 pb-4">
          <span className="text-sm font-semibold">Viary is running in the tray</span>
          <span className="text-[13px] leading-[1.45] text-[#424242]">
            Hold {s.hotkeyName} anywhere to dictate. To keep the icon in view, drag it out of the ^ menu onto the
            taskbar.
          </span>
        </div>
        <div className="grid grid-cols-2 gap-2 px-4 pb-4">
          <button
            type="button"
            onClick={() => api.closeTrayTip(true)}
            className={button + " border-[#0F5FB8] bg-[#0F5FB8] text-white hover:bg-[#0E58AB]"}
          >
            Show me
          </button>
          <button
            type="button"
            onClick={() => api.closeTrayTip(false)}
            className={button + " border-[#D1D1D1] bg-white hover:bg-[#F5F5F5]"}
          >
            Got it
          </button>
        </div>
      </div>
    </div>
  );
}
