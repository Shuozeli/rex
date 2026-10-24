// Captions (ASS, burned in by ffmpeg/libass) and joining the scenes into one video.
import { execFileSync } from "node:child_process";
import { existsSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const FONT = "Noto Sans CJK SC";
const HEADER = `[Script Info]
ScriptType: v4.00+
PlayResX: 1920
PlayResY: 1080
WrapStyle: 0
ScaledBorderAndShadow: yes

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Caption,${FONT},40,&H00FFFFFF,&H000000FF,&H38241F1F,&H00000000,0,0,0,0,100,100,0,0,3,16,0,2,40,40,40,1
Style: Card,${FONT},40,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,5,0,0,0,1
Style: Ripple,${FONT},20,&HFF000000,&H000000FF,&H00F8B48A,&HFF000000,0,0,0,0,100,100,0,0,1,4,0,7,0,0,0,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
`;

function ts(ms) {
  const t = Math.max(0, Math.round(ms / 10));
  const cs = t % 100, s = Math.floor(t / 100) % 60, m = Math.floor(t / 6000) % 60, h = Math.floor(t / 360000);
  return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}.${String(cs).padStart(2, "0")}`;
}

const esc = (s) => s.replace(/[{}\\]/g, "").replace(/\n/g, "\\N");

/** Captions from a scene's events: each caption lasts until the next one or the end; a ripple per click. */
export function sceneAss(events) {
  const end = events.find((e) => e.kind === "end")?.t ?? events.at(-1)?.t ?? 0;
  const caps = events.filter((e) => e.kind === "caption");
  const lines = caps.map((c, i) => {
    const until = caps[i + 1]?.t ?? end;
    return `Dialogue: 0,${ts(c.t)},${ts(until)},Caption,,0,0,0,,{\\fad(200,0)}${esc(c.text)}`;
  });
  for (const c of events.filter((e) => e.kind === "click")) {
    // A ring that grows and fades where the pointer clicked.
    lines.push(
      `Dialogue: 1,${ts(c.t)},${ts(c.t + 600)},Ripple,,0,0,0,,{\\pos(${c.x},${c.y})\\an5\\p1\\t(0,600,\\fscx350\\fscy350\\3a&HFF&)}m -8 0 b -8 -11 8 -11 8 0 b 8 11 -8 11 -8 0{\\p0}`,
    );
  }
  return HEADER + lines.join("\n") + "\n";
}

/** A title card: lines centered and stacked. `ok: true/false` colors a line green/red. */
export function cardAss(items, durationMs = 60000) {
  const gap = 18;
  const total = items.reduce((h, it) => h + it.size + gap, -gap);
  let y = 540 - total / 2;
  const lines = items.map((it) => {
    const color = it.ok === true ? "\\c&H95C981&" : it.ok === false ? "\\c&H828BF2&" : "";
    const line = `Dialogue: 0,${ts(0)},${ts(durationMs)},Card,,0,0,0,,{\\pos(960,${Math.round(y + it.size / 2)})\\fs${it.size}${it.bold ? "\\b1" : ""}${color}}${esc(it.text)}`;
    y += it.size + gap;
    return line;
  });
  return HEADER + lines.join("\n") + "\n";
}

const ENCODE = ["-c:v", "libx264", "-preset", "medium", "-crf", "20", "-pix_fmt", "yuv420p", "-r", "50"];

function ffmpeg(args) {
  execFileSync("ffmpeg", ["-y", "-loglevel", "error", ...args], { stdio: "inherit" });
}

/** intro + each scene with its captions + outro -> `file`. */
export function assemble({ dir, scenes, file, introMs = 4000, outroMs = 7000 }) {
  const parts = [];
  const card = (name, ms) => {
    const f = join(dir, `${name}.mp4`);
    ffmpeg(["-f", "lavfi", "-i", `color=c=0x202124:s=1920x1080:r=50:d=${ms / 1000}`, "-vf", `ass=${join(dir, `${name}.ass`)}`, ...ENCODE, f]);
    return f;
  };
  parts.push(card("intro", introMs));
  for (const s of scenes) {
    const raw = join(dir, `${s}.raw.mp4`);
    if (!existsSync(raw)) continue;
    const f = join(dir, `${s}.mp4`);
    ffmpeg(["-i", raw, "-vf", `ass=${join(dir, `${s}.ass`)}`, ...ENCODE, f]);
    parts.push(f);
  }
  parts.push(card("outro", outroMs));
  const list = join(dir, "parts.txt");
  writeFileSync(list, parts.map((p) => `file '${p}'`).join("\n") + "\n");
  ffmpeg(["-f", "concat", "-safe", "0", "-i", list, "-c", "copy", "-movflags", "+faststart", file]);
}
