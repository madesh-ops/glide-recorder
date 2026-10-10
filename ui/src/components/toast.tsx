import { create } from "zustand";
import { AlertCircle, CheckCircle2, Info, X } from "lucide-react";

type Kind = "info" | "error" | "success";
interface Toast { id: number; kind: Kind; title: string; detail?: string }

const useToasts = create<{ items: Toast[]; push(t: Omit<Toast, "id">): void; dismiss(id: number): void }>((set) => ({
  items: [],
  push: (t) => {
    const id = Date.now() + Math.random();
    set((s) => ({ items: [...s.items.slice(-3), { ...t, id }] }));
    setTimeout(() => set((s) => ({ items: s.items.filter((x) => x.id !== id) })), t.kind === "error" ? 9000 : 4000);
  },
  dismiss: (id) => set((s) => ({ items: s.items.filter((x) => x.id !== id) })),
}));

export const toast = {
  info: (title: string, detail?: string) => useToasts.getState().push({ kind: "info", title, detail }),
  success: (title: string, detail?: string) => useToasts.getState().push({ kind: "success", title, detail }),
  error: (title: string, detail?: string) => useToasts.getState().push({ kind: "error", title, detail }),
};

export function Toasts() {
  const { items, dismiss } = useToasts();
  return (
    <div className="toasts" role="status" aria-live="polite">
      {items.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`}>
          {t.kind === "error" ? (
            <AlertCircle size={16} color="var(--danger)" />
          ) : t.kind === "success" ? (
            <CheckCircle2 size={16} color="var(--success)" />
          ) : (
            <Info size={16} color="var(--accent-hover)" />
          )}
          <div className="col" style={{ gap: 2, flex: 1, minWidth: 0 }}>
            <strong style={{ fontWeight: 600 }}>{t.title}</strong>
            {t.detail && <span className="muted" style={{ fontSize: 12, wordBreak: "break-word" }}>{t.detail}</span>}
          </div>
          <button className="btn ghost icon sm" aria-label="Dismiss" onClick={() => dismiss(t.id)}>
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
