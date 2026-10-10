import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X, Copy } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

export function WindowControls() {
  const win = getCurrentWindow();
  const [max, setMax] = useState(false);
  useEffect(() => {
    let off: (() => void) | undefined;
    win.isMaximized().then(setMax).catch(() => {});
    win
      .onResized(() => win.isMaximized().then(setMax).catch(() => {}))
      .then((u) => (off = u))
      .catch(() => {});
    return () => off?.();
  }, [win]);
  return (
    <div className="wincontrols">
      <button aria-label="Minimize" onClick={() => win.minimize()}>
        <Minus size={15} strokeWidth={1.6} />
      </button>
      <button aria-label={max ? "Restore" : "Maximize"} onClick={() => win.toggleMaximize()}>
        {max ? <Copy size={12} strokeWidth={1.6} style={{ transform: "scaleX(-1)" }} /> : <Square size={12} strokeWidth={1.6} />}
      </button>
      <button className="close" aria-label="Close" onClick={() => win.close()}>
        <X size={16} strokeWidth={1.6} />
      </button>
    </div>
  );
}

/** Custom title bar: children on the left, optional centre and right content. */
export function TitleBar({ left, center, right }: { left?: ReactNode; center?: ReactNode; right?: ReactNode }) {
  return (
    <header className="titlebar" data-tauri-drag-region>
      {left}
      <div className="drag" data-tauri-drag-region />
      {center && <div className="title-center">{center}</div>}
      {right}
      <WindowControls />
    </header>
  );
}
