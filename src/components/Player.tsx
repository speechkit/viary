// Playback of a saved recording: back and forward 15 s, play/pause, the
// clock, and speed. Voice Notes adds a waveform above the controls.

import { clock } from "../lib/ipc";
import type { Playback } from "../lib/playback";
import { Icon } from "./Icons";

const round = "inline-flex size-[38px] shrink-0 items-center justify-center rounded-full disabled:opacity-40";
const iconBtn = round + " hover:bg-paper";
const playBtn = round + " bg-ink text-white hover:bg-[#3A3731]";

/** The audio element and the transport row. */
export function PlayerControls({ playback, durationMs, children }: { playback: Playback; durationMs: number; children?: React.ReactNode }) {
  const off = !playback.path || playback.failed;
  return (
    <div className="flex items-center gap-1.5">
      {playback.element}
      <button type="button" disabled={off} onClick={() => playback.skip(-15000)} aria-label="Back 15 seconds" className={iconBtn}>
        <Icon name="back15" />
      </button>
      <button
        type="button"
        disabled={off}
        onClick={playback.toggle}
        aria-label={playback.playing ? "Pause" : "Play"}
        className={playBtn}
      >
        {playback.playing ? <Icon name="pause" /> : <Icon name="play" fill="currentColor" />}
      </button>
      <button type="button" disabled={off} onClick={() => playback.skip(15000)} aria-label="Forward 15 seconds" className={iconBtn}>
        <Icon name="fwd15" />
      </button>
      <span className="ml-1.5 font-mono text-[13px]">
        {clock(playback.pos)} <span className="text-faint">/ {clock(durationMs)}</span>
      </span>
      <div className="grow" />
      {playback.failed && <span className="text-xs text-faint">This file can't be played here.</span>}
      {children}
      <button
        type="button"
        onClick={playback.cycleSpeed}
        className="ml-2 inline-flex h-[30px] items-center rounded-[9px] border border-edge bg-white px-3 text-[13px] font-medium"
      >
        {playback.speed}×
      </button>
    </div>
  );
}
