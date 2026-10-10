import { useEffect, useRef, useState } from "react";
import { Pause, Play, Square, Trash2, X, Loader2, AlertTriangle } from "lucide-react";
import { api, errorText, type SessionStatus } from "../lib/api";
import { formatBytes, formatTime } from "../lib/format";
import "../styles/recorder.css";

type Phase =
  | { kind: "countdown"; n: number }
  | { kind: "starting" }
  | { kind: "live" }
  | { kind: "stopping" }
  | { kind: "error"; message: string };

function Meter({ level }: { level: number }) {
  // Peak level → 0..1 on a perceptual (dB) scale.
  const db = 20 * Math.log10(Math.max(level, 1e-4));
  const v = Math.min(1, Math.max(0, (db + 50) / 50));
  return (
    <div className="meter" aria-hidden>
      <div style={{ height: `${v * 100}%` }} />
    </div>
  );
}

export function RecorderBar() {
  const [phase, setPhase] = useState<Phase>({ kind: "starting" });
  const [status, setStatus] = useState<SessionStatus | null>(null);
  const stopping = useRef(false);
  const cancelled = useRef(false);

  // Countdown, then start.
  useEffect(() => {
    let timer: number | undefined;
    (async () => {
      const pending = await api.pendingRecording().catch(() => null);
      if (!pending) {
        // Already running (e.g. window reloaded): just show controls.
        const st = await api.recordingStatus().catch(() => null);
        setPhase(st ? { kind: "live" } : { kind: "error", message: "Nothing to record." });
        return;
      }
      let n = pending.countdown;
      const begin = async () => {
        if (cancelled.current) return;
        setPhase({ kind: "starting" });
        try {
          await api.startRecording();
          setPhase({ kind: "live" });
        } catch (e) {
          setPhase({ kind: "error", message: errorText(e) });
        }
      };
      if (n <= 0) return begin();
      setPhase({ kind: "countdown", n });
      timer = window.setInterval(() => {
        n -= 1;
        if (n <= 0) {
          window.clearInterval(timer);
          begin();
        } else {
          setPhase({ kind: "countdown", n });
        }
      }, 1000);
    })();
    return () => window.clearInterval(timer);
  }, []);

  // Poll status while live.
  useEffect(() => {
    if (phase.kind !== "live") return;
    const id = window.setInterval(async () => {
      const st = await api.recordingStatus().catch(() => null);
      if (!st) return;
      setStatus(st);
      if (st.needsStop && !stopping.current) stop();
    }, 200);
    return () => window.clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase.kind]);

  const stop = async () => {
    if (stopping.current) return;
    stopping.current = true;
    setPhase({ kind: "stopping" });
    try {
      await api.stopRecording(); // Rust closes this window and opens the editor.
    } catch (e) {
      setPhase({ kind: "error", message: errorText(e) });
      stopping.current = false;
    }
  };

  const togglePause = async () => {
    if (!status) return;
    try {
      if (status.state.state === "paused") await api.resumeRecording();
      else await api.pauseRecording();
      setStatus(await api.recordingStatus());
    } catch {
      /* state changed underneath us; the next poll corrects the UI */
    }
  };

  const discard = async () => {
    stopping.current = true;
    await api.discardRecording().catch(() => {});
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (phase.kind === "live") {
        if (e.key === " ") togglePause();
        if (e.key === "Enter") stop();
      }
      if (phase.kind === "countdown" && e.key === "Escape") {
        cancelled.current = true;
        api.cancelPending();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const paused = status?.state.state === "paused";

  return (
    <div className="recbar" data-tauri-drag-region>
      {phase.kind === "countdown" && (
        <>
          <div className="countdown" data-tauri-drag-region>
            <span className="count" key={phase.n}>{phase.n}</span>
            <span className="muted" data-tauri-drag-region>Recording starts…</span>
          </div>
          <button
            className="rb-btn"
            title="Cancel (Esc)"
            onClick={() => {
              cancelled.current = true;
              api.cancelPending();
            }}
          >
            <X size={16} />
          </button>
        </>
      )}
      {(phase.kind === "starting" || phase.kind === "stopping") && (
        <div className="countdown" data-tauri-drag-region>
          <Loader2 size={16} className="spin" />
          <span className="muted">{phase.kind === "starting" ? "Starting capture…" : "Saving recording…"}</span>
        </div>
      )}
      {phase.kind === "live" && (
        <>
          <div className={`rec-indicator ${paused ? "paused" : ""}`} data-tauri-drag-region />
          <div className="timer" data-tauri-drag-region>
            <span className="mono">{formatTime(status?.elapsed ?? 0)}</span>
            <span className="faint size" data-tauri-drag-region>{status ? formatBytes(status.bytes) : ""}</span>
          </div>
          {status?.micLevel != null && <Meter level={status.micLevel} />}
          {status?.systemLevel != null && <Meter level={status.systemLevel} />}
          {status && status.warnings.length > 0 && (
            <span className="warn" title={status.warnings.join("\n")}><AlertTriangle size={14} /></span>
          )}
          <div className="spacer" data-tauri-drag-region />
          <button className="rb-btn" title={paused ? "Resume (Space)" : "Pause (Space)"} onClick={togglePause}>
            {paused ? <Play size={16} fill="currentColor" /> : <Pause size={16} fill="currentColor" />}
          </button>
          <button className="rb-btn stop" title="Stop and edit (Enter)" onClick={stop}>
            <Square size={13} fill="currentColor" />
          </button>
          <button className="rb-btn" title="Discard recording" onClick={discard}>
            <Trash2 size={15} />
          </button>
        </>
      )}
      {phase.kind === "error" && (
        <>
          <AlertTriangle size={16} color="var(--danger)" />
          <span className="error-text" title={phase.message}>{phase.message}</span>
          <button className="rb-btn" title="Close" onClick={() => api.cancelPending()}>
            <X size={16} />
          </button>
        </>
      )}
    </div>
  );
}
