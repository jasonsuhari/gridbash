import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:net";
import { copyFileSync, existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "../..");

function argValue(name, fallback) {
  const index = process.argv.indexOf(name);
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback;
}

const output = resolve(argValue("--output", join(repoRoot, "docs/assets/gridbash-openvid-demo.mp4")));
const poster = resolve(argValue("--poster", join(repoRoot, "docs/assets/gridbash-openvid-demo-poster.png")));
const fps = Number(argValue("--fps", "30"));
const duration = Number(argValue("--duration", "18"));
const keepFrames = process.argv.includes("--keep-frames");
const html = pathToFileURL(join(__dirname, "openvid-gridbash-demo.html")).href;
const frameRoot = join(process.env.TEMP || process.env.TMP || repoRoot, "gridbash-openvid-demo-frames");
const width = 1920;
const height = 1080;

function findChrome() {
  const candidates = [
    process.env.CHROME_PATH,
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
  ].filter(Boolean);
  const found = candidates.find((candidate) => existsSync(candidate));
  if (!found) {
    throw new Error("Google Chrome was not found. Install Chrome or set CHROME_PATH.");
  }
  return found;
}

async function getFreePort() {
  return new Promise((resolvePort, reject) => {
    const server = createServer();
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = address.port;
      server.close(() => resolvePort(port));
    });
    server.on("error", reject);
  });
}

async function fetchJson(url, timeoutMs = 30000) {
  const started = Date.now();
  let lastError;
  while (Date.now() - started < timeoutMs) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return await response.json();
      }
    } catch (error) {
      lastError = error;
    }
    await delay(100);
  }
  throw lastError || new Error(`Timed out fetching ${url}`);
}

function connectCdp(wsUrl) {
  const ws = new WebSocket(wsUrl);
  let nextId = 1;
  const pending = new Map();
  const listeners = new Set();

  ws.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolveMessage, rejectMessage } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) {
        rejectMessage(new Error(message.error.message));
      } else {
        resolveMessage(message.result);
      }
      return;
    }
    for (const listener of listeners) {
      listener(message);
    }
  });

  const opened = new Promise((resolveOpen, rejectOpen) => {
    ws.addEventListener("open", resolveOpen, { once: true });
    ws.addEventListener("error", rejectOpen, { once: true });
  });

  function send(method, params = {}, sessionId) {
    const id = nextId++;
    const payload = { id, method, params };
    if (sessionId) {
      payload.sessionId = sessionId;
    }
    ws.send(JSON.stringify(payload));
    return new Promise((resolveMessage, rejectMessage) => {
      pending.set(id, { resolveMessage, rejectMessage });
    });
  }

  async function waitFor(predicate, timeoutMs = 30000) {
    const started = Date.now();
    return new Promise((resolveWait, rejectWait) => {
      const listener = (message) => {
        if (predicate(message)) {
          listeners.delete(listener);
          resolveWait(message);
        }
      };
      listeners.add(listener);
      const timer = setInterval(() => {
        if (Date.now() - started > timeoutMs) {
          clearInterval(timer);
          listeners.delete(listener);
          rejectWait(new Error("Timed out waiting for CDP event"));
        }
      }, 100);
    });
  }

  return { ws, opened, send, waitFor };
}

async function removeDirWithRetry(path, { warn = true } = {}) {
  if (!existsSync(path)) {
    return;
  }
  for (let attempt = 0; attempt < 10; attempt += 1) {
    try {
      rmSync(path, { recursive: true, force: true });
      return;
    } catch (error) {
      if (attempt === 9) {
        if (warn) {
          console.warn(`Warning: could not remove ${path}: ${error.message}`);
        }
        return;
      }
      await delay(250);
    }
  }
}

function killChromeByProfile(userDataDir) {
  if (process.platform !== "win32") {
    return;
  }
  const needle = userDataDir.replace(/'/g, "''");
  spawnSync("powershell", [
    "-NoProfile",
    "-NonInteractive",
    "-Command",
    `$needle = '${needle}'; Get-CimInstance Win32_Process -Filter "Name = 'chrome.exe'" | Where-Object { $_.CommandLine -like "*$needle*" } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }`,
  ], { stdio: "ignore" });
}

async function encodeVideo(totalFrames) {
  mkdirSync(dirname(output), { recursive: true });
  mkdirSync(dirname(poster), { recursive: true });

  const posterIndex = Math.max(0, Math.min(totalFrames - 1, Math.round(fps * 10)));
  const posterFrame = join(frameRoot, `frame_${String(posterIndex).padStart(5, "0")}.png`);
  copyFileSync(posterFrame, poster);

  const ffmpegArgs = [
    "-y",
    "-hide_banner",
    "-loglevel",
    "error",
    "-framerate",
    String(fps),
    "-i",
    join(frameRoot, "frame_%05d.png"),
    "-c:v",
    "libx264",
    "-preset",
    "medium",
    "-crf",
    "21",
    "-pix_fmt",
    "yuv420p",
    "-movflags",
    "+faststart",
    output,
  ];

  await new Promise((resolveEncode, rejectEncode) => {
    const ffmpeg = spawn("ffmpeg", ffmpegArgs, { stdio: "inherit" });
    ffmpeg.on("exit", (code) => {
      if (code === 0) {
        resolveEncode();
      } else {
        rejectEncode(new Error(`ffmpeg exited with code ${code}`));
      }
    });
  });
}

async function main() {
  const totalFrames = Math.round(fps * duration);
  if (existsSync(frameRoot)) {
    rmSync(frameRoot, { recursive: true, force: true });
  }
  mkdirSync(frameRoot, { recursive: true });

  const chromePath = findChrome();
  const port = await getFreePort();
  const userDataDir = join(process.env.TEMP || process.env.TMP || repoRoot, `gridbash-demo-chrome-${process.pid}`);
  const chrome = spawn(chromePath, [
    "--headless=new",
    "--disable-gpu",
    "--disable-background-networking",
    "--disable-breakpad",
    "--disable-component-update",
    "--disable-crash-reporter",
    "--disable-default-apps",
    "--disable-extensions",
    "--hide-scrollbars",
    "--mute-audio",
    "--no-first-run",
    "--no-default-browser-check",
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${userDataDir}`,
    `--window-size=${width},${height}`,
    "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  const chromeExited = new Promise((resolveExit) => chrome.once("exit", resolveExit));

  chrome.stderr.on("data", (chunk) => {
    const text = String(chunk);
    if (!text.includes("DevTools")) {
      process.stderr.write(text);
    }
  });

  let cdp;
  try {
    const version = await fetchJson(`http://127.0.0.1:${port}/json/version`);
    cdp = connectCdp(version.webSocketDebuggerUrl);
    await cdp.opened;

    const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
    const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
    await cdp.send("Page.enable", {}, sessionId);
    await cdp.send("Runtime.enable", {}, sessionId);
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width,
      height,
      deviceScaleFactor: 1,
      mobile: false,
      screenWidth: width,
      screenHeight: height,
    }, sessionId);

    await cdp.send("Page.navigate", { url: html }, sessionId);

    let isReady = false;
    for (let i = 0; i < 300; i += 1) {
      const ready = await cdp.send("Runtime.evaluate", {
        expression: "Boolean(window.__gridbashDemoReady)",
        returnByValue: true,
      }, sessionId);
      if (ready.result.value) {
        isReady = true;
        break;
      }
      await delay(100);
    }
    if (!isReady) {
      throw new Error("Timed out waiting for demo page readiness");
    }

    for (let i = 0; i < totalFrames; i += 1) {
      const t = i / fps;
      await cdp.send("Runtime.evaluate", {
        expression: `window.__setDemoTime(${t.toFixed(5)})`,
        awaitPromise: true,
      }, sessionId);
      const result = await cdp.send("Page.captureScreenshot", {
        format: "png",
        fromSurface: true,
        captureBeyondViewport: false,
      }, sessionId);
      writeFileSync(join(frameRoot, `frame_${String(i).padStart(5, "0")}.png`), Buffer.from(result.data, "base64"));
      if (i % fps === 0) {
        console.log(`Captured ${i}/${totalFrames} frames`);
      }
    }

    await encodeVideo(totalFrames);
    console.log(`Wrote ${output}`);
    console.log(`Wrote ${poster}`);
  } finally {
    if (cdp) {
      try {
        await cdp.send("Browser.close");
      } catch {
        // Chrome may already be gone.
      }
      try {
        cdp.ws.close();
      } catch {
        // Ignore teardown errors.
      }
    }
    if (process.platform === "win32" && chrome.pid) {
      spawnSync("taskkill", ["/PID", String(chrome.pid), "/T", "/F"], { stdio: "ignore" });
      killChromeByProfile(userDataDir);
    } else {
      chrome.kill();
    }
    chrome.stderr.destroy();
    await Promise.race([chromeExited, delay(5000)]);
    if (!keepFrames) {
      await removeDirWithRetry(frameRoot);
    }
    await removeDirWithRetry(userDataDir, { warn: false });
  }
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error(error);
    process.exit(1);
  });
