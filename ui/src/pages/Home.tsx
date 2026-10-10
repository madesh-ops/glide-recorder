import { Clapperboard, FolderOpen, LayoutGrid, Settings as SettingsIcon, AlertTriangle } from "lucide-react";
import type { AppInfo } from "../lib/api";
import { api } from "../lib/api";
import { TitleBar } from "../components/TitleBar";
import { Projects } from "./Projects";
import { NewRecording } from "./NewRecording";
import { Settings } from "./Settings";
import "../styles/home.css";

export type HomeTab = "projects" | "record" | "settings";

export function Home({
  info,
  tab,
  onTab,
  onOpen,
}: {
  info: AppInfo | null;
  tab: HomeTab;
  onTab: (t: HomeTab) => void;
  onOpen: (dir: string) => void;
}) {
  return (
    <>
      <TitleBar
        left={
          <div className="brand" data-tauri-drag-region>
            <img src="/logo.svg" alt="" draggable={false} />
            Glide
          </div>
        }
      />
      <div className="home">
        <aside className="home-sidebar">
          <button className="btn record lg new-rec" onClick={() => onTab("record")}>
            <span className="rec-dot" /> New recording
          </button>
          <nav className="nav">
            <button className={tab === "projects" ? "on" : ""} onClick={() => onTab("projects")}>
              <LayoutGrid size={16} /> Projects
            </button>
            <button className={tab === "settings" ? "on" : ""} onClick={() => onTab("settings")}>
              <SettingsIcon size={16} /> Settings
            </button>
          </nav>
          <div className="spacer" />
          {info?.ffmpegError && (
            <div className="sidebar-warning">
              <AlertTriangle size={14} /> FFmpeg not found — recording and export are unavailable.
            </div>
          )}
          {info && (
            <button className="folder-link" title={info.outputDir} onClick={() => api.revealPath(info.outputDir).catch(() => {})}>
              <FolderOpen size={14} />
              <span>Recordings folder</span>
            </button>
          )}
          <div className="faint version">
            <Clapperboard size={12} /> Glide {info?.version ?? ""}
          </div>
        </aside>
        <main className="home-main">
          {tab === "projects" && <Projects onOpen={onOpen} onNew={() => onTab("record")} />}
          {tab === "record" && <NewRecording info={info} onCancel={() => onTab("projects")} />}
          {tab === "settings" && <Settings info={info} />}
        </main>
      </div>
    </>
  );
}
