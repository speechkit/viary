// Playback of a saved recording through an audio element: position,
// play/pause, skipping, and speed. Player.tsx draws the controls.

import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";

const SPEEDS = [1, 1.5, 2];

export interface Playback {
  /** Where playback is, in ms. */
  pos: number;
  playing: boolean;
  /** Moves to `ms`, and plays from there unless `play` is false. */
  seek: (ms: number, play?: boolean) => void;
  toggle: () => void;
  skip: (deltaMs: number) => void;
  speed: number;
  cycleSpeed: () => void;
  /** The file to play, or null when it is gone. */
  path: string | null;
  /** The webview cannot play the file, such as a video in Matroska. */
  failed: boolean;
  /** The audio element, wired to this state. */
  element: React.ReactNode;
}

/** Playback of the file at `path`; starts over when the file changes. */
export function usePlayback(path: string | null, durationMs: number): Playback {
  const audio = useRef<HTMLAudioElement>(null);
  const [pos, setPos] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setPos(0);
    setPlaying(false);
    setFailed(false);
  }, [path]);

  useEffect(() => {
    if (audio.current) audio.current.playbackRate = speed;
  }, [speed, path]);

  // timeupdate fires only a few times a second; follow the audio every
  // frame while playing, so the highlight keeps up with the voice.
  useEffect(() => {
    if (!playing) return;
    let frame = 0;
    const tick = () => {
      if (audio.current) setPos(audio.current.currentTime * 1000);
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [playing]);

  const seek = (ms: number, play = true) => {
    const at = Math.min(Math.max(0, ms), durationMs);
    setPos(at);
    const el = audio.current;
    if (!el) return;
    el.currentTime = at / 1000;
    if (play) el.play().catch(console.error);
  };

  return {
    pos,
    playing,
    seek,
    toggle: () => {
      const el = audio.current;
      if (!el) return;
      if (!el.paused) return el.pause();
      if (el.ended) el.currentTime = 0;
      el.play().catch(console.error);
    },
    skip: (delta) => seek(pos + delta, playing),
    speed,
    cycleSpeed: () => setSpeed((s) => SPEEDS[(SPEEDS.indexOf(s) + 1) % SPEEDS.length]),
    path,
    failed,
    // The element's own events decide `playing`, so the end of the file,
    // the keyboard and the buttons all agree.
    element: path && (
      <audio
        ref={audio}
        src={convertFileSrc(path)}
        preload="metadata"
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => setPos(durationMs)}
        onError={() => setFailed(true)}
      />
    ),
  };
}
