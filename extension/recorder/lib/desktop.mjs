// The X desktop inside the recorder container: real input (xdotool), screenshots, and screen recording
// (ffmpeg x11grab, one frame every 20 ms).
import { execFileSync, spawn } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const env = { ...process.env, DISPLAY: process.env.DISPLAY ?? ":99" };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const xdo = (...args) => execFileSync("xdotool", args.map(String), { env });

export function mousePosition() {
  const out = execFileSync("xdotool", ["getmouselocation", "--shell"], { env, encoding: "utf8" });
  const get = (k) => Number(out.match(new RegExp(`^${k}=(\\d+)`, "m"))[1]);
  return { x: get("X"), y: get("Y") };
}

/** Moves the pointer along an eased path, one step every 20 ms. */
export async function moveTo(x, y, ms = 600) {
  const from = mousePosition();
  const steps = Math.max(1, Math.round(ms / 20));
  for (let i = 1; i <= steps; i++) {
    const k = i / steps;
    const e = k * k * (3 - 2 * k);
    xdo("mousemove", Math.round(from.x + (x - from.x) * e), Math.round(from.y + (y - from.y) * e));
    await sleep(20);
  }
}

export async function click(x, y, ms = 600) {
  await moveTo(x, y, ms);
  await sleep(120);
  xdo("click", 1);
}

/** Wheel scroll at the pointer: `notches` clicks of button 5 (down) or 4 (up), spread over `ms`. */
export async function wheel(notches, ms = 1000) {
  const button = notches > 0 ? 5 : 4;
  const n = Math.abs(notches);
  for (let i = 0; i < n; i++) {
    xdo("click", button);
    await sleep(ms / n);
  }
}

export function key(...keys) {
  xdo("key", "--clearmodifiers", ...keys);
}

/** Types text like a person (a delay per character). */
export function type(text, delayMs = 35) {
  xdo("type", "--delay", delayMs, text);
}

export function screenshot(file) {
  execFileSync("import", ["-window", "root", file], { env });
  return file;
}

/** Center of the extension's toolbar icon, found on a fresh screenshot. */
export function locateIcon(iconPng, tmpFile = "/tmp/rex-locate.png") {
  screenshot(tmpFile);
  const out = execFileSync("python3", [join(here, "locate_icon.py"), tmpFile, iconPng], { encoding: "utf8" }).trim();
  const [x, y, score] = out.split(/\s+/).map(Number);
  return { x, y, score };
}

/**
 * Records the whole screen at 50 fps until stop(). `t0` is the wall time (ms) of the first frame, calibrated
 * from ffmpeg's progress reports, so events logged with Date.now() can be placed on the video's timeline.
 */
export function startRecording(file, { size = "1920x1080", fps = 50 } = {}) {
  const ff = spawn(
    "ffmpeg",
    ["-y", "-loglevel", "error", "-f", "x11grab", "-framerate", String(fps), "-video_size", size, "-draw_mouse", "1", "-i", env.DISPLAY,
     "-c:v", "libx264", "-preset", "ultrafast", "-crf", "18", "-pix_fmt", "yuv420p", "-progress", "pipe:1", "-stats_period", "0.2", file],
    { env, stdio: ["pipe", "pipe", "inherit"] },
  );
  const rec = { file, t0: null, ff };
  let buf = "";
  ff.stdout.on("data", (d) => {
    buf += d;
    let i;
    while ((i = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, i);
      buf = buf.slice(i + 1);
      const m = line.match(/^out_time_us=(\d+)/);
      // The first report with real time in it: the first frame was taken that long before now.
      if (m && rec.t0 === null && Number(m[1]) > 0) rec.t0 = Date.now() - Number(m[1]) / 1000;
    }
  });
  rec.ready = (async () => {
    for (let i = 0; i < 100 && rec.t0 === null; i++) await sleep(50);
    if (rec.t0 === null) throw new Error("ffmpeg did not start recording");
  })();
  rec.stop = () =>
    new Promise((resolve) => {
      ff.on("exit", resolve);
      ff.stdin.write("q");
      ff.stdin.end();
    });
  return rec;
}
