import { useCallback, useEffect, useState } from "react";
import { api, errorText, on, type AppInfo } from "./lib/api";
import { Home, type HomeTab } from "./pages/Home";
import { Editor } from "./pages/editor/Editor";
import { Toasts, toast } from "./components/toast";

type Route = { page: "home"; tab: HomeTab } | { page: "editor"; dir: string };

function initialRoute(): Route {
  // Dev-only deep links for the mocked UI harness (?mock&page=editor|record|settings).
  if (import.meta.env.DEV) {
    const page = new URLSearchParams(location.search).get("page");
    if (page === "editor") return { page: "editor", dir: "mock" };
    if (page === "record" || page === "settings") return { page: "home", tab: page };
  }
  return { page: "home", tab: "projects" };
}

export function App() {
  const [route, setRoute] = useState<Route>(initialRoute);
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    api.appInfo().then(setInfo).catch((e) => toast.error("Startup problem", errorText(e)));
  }, []);

  useEffect(() => {
    const un = on<{ dir?: string | null; error?: string | null }>("recording-finished", (p) => {
      if (p.dir) setRoute({ page: "editor", dir: p.dir });
      else toast.error("Recording failed", p.error ?? "Unknown error");
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const openProject = useCallback((dir: string) => setRoute({ page: "editor", dir }), []);

  return (
    <div className="app">
      {route.page === "home" ? (
        <Home info={info} tab={route.tab} onTab={(tab) => setRoute({ page: "home", tab })} onOpen={openProject} />
      ) : (
        <Editor dir={route.dir} onClose={() => setRoute({ page: "home", tab: "projects" })} />
      )}
      <Toasts />
    </div>
  );
}
