import "@fontsource/geist-sans/400.css";
import "@fontsource/geist-sans/500.css";
import "@fontsource/geist-sans/600.css";
import "@fontsource/geist-mono/400.css";
import "@fontsource/geist-mono/500.css";
import "@fontsource/newsreader/400.css";
import "@fontsource/newsreader/500.css";
import "./styles.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Pill } from "./windows/Pill";
import { Popover } from "./windows/Popover";
import { MainWindow } from "./windows/MainWindow";
import { Setup } from "./windows/Setup";
import { TrayTip } from "./windows/TrayTip";

// One bundle serves every window; the window's label picks the view.
const label = getCurrentWebviewWindow().label;
document.documentElement.dataset.window = label;

const view =
  label === "pill" ? (
    <Pill />
  ) : label === "popover" ? (
    <Popover />
  ) : label === "setup" ? (
    <Setup />
  ) : label === "tip" ? (
    <TrayTip />
  ) : (
    <MainWindow />
  );

createRoot(document.getElementById("root")!).render(<StrictMode>{view}</StrictMode>);
