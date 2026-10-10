import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles/theme.css";
import { App } from "./App";
import { RecorderBar } from "./pages/RecorderBar";

// The floating recording bar is a separate window loaded at #/recorder.
const isRecorder = window.location.hash.startsWith("#/recorder");
if (isRecorder) document.documentElement.classList.add("recorder-window");

// Disable the browser context menu outside text inputs (desktop app feel).
window.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement;
  if (!t.closest("input, textarea")) e.preventDefault();
});

async function boot() {
  // Dev-only: render with mocked IPC in a normal browser (see src/dev/mock.ts).
  if (import.meta.env.DEV && new URLSearchParams(location.search).has("mock")) {
    await (await import("./dev/mock")).installMocks();
  }
  createRoot(document.getElementById("root")!).render(
    <StrictMode>{isRecorder ? <RecorderBar /> : <App />}</StrictMode>,
  );
}
boot();
