import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import { createReadStream, existsSync, statSync } from "node:fs";
import { resolve, normalize, sep, extname } from "node:path";

/**
 * Dev-server only: serve UI fixtures (which may contain real screen recordings)
 * from ui/dev-fixtures at /__mock. They are never copied into production builds.
 */
function devFixtures(): Plugin {
  const root = resolve(__dirname, "ui/dev-fixtures");
  const types: Record<string, string> = { ".json": "application/json", ".mp4": "video/mp4", ".png": "image/png", ".jpg": "image/jpeg", ".wav": "audio/wav" };
  return {
    name: "glide-dev-fixtures",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/__mock", (req, res, next) => {
        const rel = decodeURIComponent((req.url ?? "/").split("?")[0]);
        const file = normalize(resolve(root, "." + rel));
        if (!file.startsWith(root + sep) || !existsSync(file) || !statSync(file).isFile()) return next();
        const size = statSync(file).size;
        const type = types[extname(file).toLowerCase()] ?? "application/octet-stream";
        const range = /bytes=(\d+)-(\d*)/.exec(req.headers.range ?? "");
        if (range) {
          const start = Number(range[1]);
          const end = range[2] ? Number(range[2]) : size - 1;
          res.writeHead(206, { "Content-Type": type, "Content-Range": `bytes ${start}-${end}/${size}`, "Accept-Ranges": "bytes", "Content-Length": end - start + 1 });
          createReadStream(file, { start, end }).pipe(res);
        } else {
          res.writeHead(200, { "Content-Type": type, "Content-Length": size, "Accept-Ranges": "bytes" });
          createReadStream(file).pipe(res);
        }
      });
    },
  };
}

// Tauri expects a fixed dev port and serves the built files from ui/dist.
export default defineConfig({
  root: "ui",
  plugins: [react(), devFixtures()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    sourcemap: false,
  },
  test: {
    root: ".",
    include: ["ui/src/**/*.test.ts"],
  },
});
