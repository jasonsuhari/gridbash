import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { homeDir } from "@tauri-apps/api/path";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";

const BUSY_FOR_MS = 1500;
const INFO_POLL_MS = 1000;
const STARTING_PANES = 4;

interface PaneInfo {
  program: string | null;
  folder: string | null;
}

interface Pane {
  id: number;
  element: HTMLElement;
  name: HTMLElement;
  program: HTMLElement;
  folder: HTMLElement;
  terminal: Terminal;
  fit: FitAddon;
  customName: string | null;
  lastOutputAt: number;
  exited: boolean;
}

const log = (message: string) => invoke("log_frontend", { message }).catch(() => {});
window.addEventListener("error", (event) => log(`error: ${event.message} at ${event.filename}:${event.lineno}`));
window.addEventListener("unhandledrejection", (event) => log(`unhandled rejection: ${String(event.reason)}`));
const startedAt = performance.now();

const grid = document.getElementById("grid")!;
const panes: Pane[] = [];
let focused: Pane | null = null;
const home = await homeDir().catch(() => null);

const terminalTheme = {
  background: "rgb(11, 15, 20)",
  foreground: "rgb(230, 237, 243)",
  cursor: "rgb(196, 58, 70)",
  cursorAccent: "rgb(11, 15, 20)",
  selectionBackground: "rgba(126, 231, 235, 0.35)",
};

function shortFolder(folder: string | null): string {
  if (!folder) return "";
  return home && folder.startsWith(home) ? "~" + folder.slice(home.length) : folder;
}

function layout() {
  const columns = Math.ceil(Math.sqrt(panes.length));
  const rows = Math.max(1, Math.ceil(panes.length / columns));
  grid.style.gridTemplateColumns = `repeat(${columns}, minmax(0, 1fr))`;
  grid.style.gridTemplateRows = `repeat(${rows}, minmax(0, 1fr))`;
  panes.forEach((pane, index) => {
    (pane.element.querySelector(".pane-number") as HTMLElement).textContent = String(index + 1);
  });
  requestAnimationFrame(() => panes.forEach(fitPane));
}

function fitPane(pane: Pane) {
  pane.fit.fit();
  invoke("resize_pane", { id: pane.id, rows: pane.terminal.rows, cols: pane.terminal.cols }).catch(() => {});
}

function focus(pane: Pane) {
  focused?.element.classList.remove("focused");
  focused = pane;
  pane.element.classList.add("focused");
  pane.terminal.focus();
}

function render(pane: Pane, info: PaneInfo | null) {
  const program = info?.program ?? "";
  pane.program.textContent = program;
  pane.program.style.display = program ? "" : "none";
  pane.folder.textContent = shortFolder(info?.folder ?? null);
  if (!pane.customName && pane.name.getAttribute("contenteditable") !== "true") {
    const folderName = info?.folder?.split("/").filter(Boolean).pop();
    pane.name.textContent = folderName ?? "pane";
  }
  pane.element.classList.toggle("busy", Date.now() - pane.lastOutputAt < BUSY_FOR_MS);
  pane.element.classList.toggle("exited", pane.exited);
}

function startRename(pane: Pane) {
  pane.name.setAttribute("contenteditable", "true");
  pane.name.focus();
  document.getSelection()?.selectAllChildren(pane.name);
  const finish = (keep: boolean) => {
    pane.name.removeAttribute("contenteditable");
    const typed = pane.name.textContent?.trim() ?? "";
    if (keep) pane.customName = typed || null;
    pane.name.removeEventListener("keydown", onKey);
    pane.name.removeEventListener("blur", onBlur);
    pane.terminal.focus();
  };
  const onKey = (event: KeyboardEvent) => {
    if (event.key === "Enter") {
      event.preventDefault();
      finish(true);
    } else if (event.key === "Escape") {
      event.preventDefault();
      pane.name.textContent = pane.customName ?? "";
      finish(false);
    }
  };
  const onBlur = () => finish(true);
  pane.name.addEventListener("keydown", onKey);
  pane.name.addEventListener("blur", onBlur);
}

async function openPane(cwd: string | null = null) {
  const element = document.createElement("section");
  element.className = "pane";
  element.innerHTML = `
    <div class="pane-header">
      <span class="pane-number"></span>
      <span class="pane-activity"></span>
      <span class="pane-name">pane</span>
      <span class="pane-program"></span>
      <span class="pane-folder"></span>
    </div>
    <div class="pane-body"></div>`;
  grid.appendChild(element);

  const terminal = new Terminal({
    fontFamily: '"SF Mono", Menlo, monospace',
    fontSize: 13,
    lineHeight: 1.15,
    cursorBlink: true,
    macOptionIsMeta: true,
    macOptionClickForcesSelection: true,
    scrollback: 10000,
    allowProposedApi: true,
    theme: terminalTheme,
  });
  const fit = new FitAddon();
  terminal.loadAddon(fit);
  terminal.open(element.querySelector(".pane-body") as HTMLElement);
  try {
    terminal.loadAddon(new WebglAddon());
  } catch (error) {
    log(`WebGL renderer unavailable, using the DOM renderer: ${String(error)}`);
  }
  fit.fit();

  const output = new Channel<ArrayBuffer>();
  const pane: Pane = {
    id: 0,
    element,
    name: element.querySelector(".pane-name") as HTMLElement,
    program: element.querySelector(".pane-program") as HTMLElement,
    folder: element.querySelector(".pane-folder") as HTMLElement,
    terminal,
    fit,
    customName: null,
    lastOutputAt: 0,
    exited: false,
  };
  output.onmessage = (bytes) => {
    if (pane.lastOutputAt === 0) {
      log(`pane ${pane.id} first output: ${Object.prototype.toString.call(bytes)} after ${Math.round(performance.now() - startedAt)}ms`);
    }
    terminal.write(new Uint8Array(bytes));
    pane.lastOutputAt = Date.now();
  };
  pane.id = await invoke<number>("open_pane", { cwd, rows: terminal.rows, cols: terminal.cols, output });
  terminal.onData((data) => invoke("write_pane", { id: pane.id, data }).catch(() => {}));
  element.addEventListener("mousedown", () => focus(pane));
  element.querySelector(".pane-header")!.addEventListener("dblclick", () => startRename(pane));
  panes.push(pane);
  layout();
  focus(pane);
  return pane;
}

function closePane(pane: Pane) {
  invoke("close_pane", { id: pane.id }).catch(() => {});
  pane.terminal.dispose();
  pane.element.remove();
  const index = panes.indexOf(pane);
  panes.splice(index, 1);
  if (focused === pane) focused = null;
  if (panes.length === 0) {
    openPane();
    return;
  }
  layout();
  focus(panes[Math.min(index, panes.length - 1)]);
}

async function pollInfo() {
  await Promise.all(
    panes.map(async (pane) => {
      const info = await invoke<PaneInfo | null>("pane_info", { id: pane.id }).catch(() => null);
      render(pane, info);
    }),
  );
}

window.addEventListener(
  "keydown",
  (event) => {
    if (!event.metaKey) return;
    const digit = Number(event.key);
    if (event.key === "t") {
      openPane(null);
    } else if (event.key === "w" && focused) {
      closePane(focused);
    } else if (event.key === "r" && focused) {
      startRename(focused);
    } else if (digit >= 1 && digit <= 9 && panes[digit - 1]) {
      focus(panes[digit - 1]);
    } else {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
  },
  true,
);

window.addEventListener("resize", () => panes.forEach(fitPane));

await listen<{ id: number }>("pane-exited", ({ payload }) => {
  const pane = panes.find((candidate) => candidate.id === payload.id);
  if (!pane) return;
  pane.exited = true;
  pane.terminal.write("\r\n\x1b[2m[process exited]\x1b[0m\r\n");
});

for (let opened = 0; opened < STARTING_PANES; opened++) await openPane();
focus(panes[0]);
setInterval(pollInfo, INFO_POLL_MS);
pollInfo();
