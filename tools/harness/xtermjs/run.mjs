// Runs a program in xterm.js's headless terminal on a pty, for the harness.
//
//   node run.mjs COLS ROWS CWD PROGRAM [ARGS...]
//
// It prints one JSON object per line on standard output. {"ready":true} comes once the program
// runs. Each line read on standard input is a JSON request:
//
//   {"read":true}       the screen, as {"ansi": rows with SGR, "cursor": [x, y], "visible": bool}
//   {"send":"1b5b41"}   bytes, in hex, to the program as if typed
//   {"mouse":"press","button":"left","x":3,"y":2}
//                       a mouse event over a cell counted from 0, which xterm.js's own mouse
//                       service reports by the modes the program set. The wheel is "scroll"
//                       with "up" or "down".
//   {"quit":true}       ends the program and this script
//
// Answers to the program's queries go back to it, as a terminal sends them.

import { createInterface } from "node:readline";
import headless from "@xterm/headless";
import pty from "node-pty";

const [cols, rows] = [Number(process.argv[2]), Number(process.argv[3])];
const [cwd, program, ...args] = process.argv.slice(4);

const term = new headless.Terminal({ cols, rows, scrollback: 0, allowProposedApi: true });
// VS Code's terminal says it has true colour, as programs can't ask xterm.js.
const env = { ...process.env, COLORTERM: "truecolor" };
const child = pty.spawn(program, args, { name: "xterm-256color", cols, rows, cwd, env, encoding: null });
child.onData((data) => term.write(data));
term.onData((data) => child.write(data));
term.onBinary((data) => child.write(Buffer.from(data, "binary")));
child.onExit(() => {});

const reply = (value) => process.stdout.write(`${JSON.stringify(value)}\n`);

// xterm.js keeps a colour's mode in the top bits: 16 colours, 256 colours or RGB.
const P16 = 0x1000000;

function color(cell, isRgb, isPalette, value, mode, base, bright, extended) {
  if (isRgb) {
    return [`${extended};2;${(value >> 16) & 255};${(value >> 8) & 255};${value & 255}`];
  }
  if (isPalette) {
    if (mode === P16) {
      return [String(value < 8 ? base + value : bright + value - 8)];
    }
    return [`${extended};5;${value}`];
  }
  return [];
}

// What xterm.js 6.0 keeps outside its public API: the underline's style and colour, and the link.
// The public isUnderline() is true for a linked cell even without SGR 4.
function extended(cell) {
  // A reused cell keeps the last extended attributes it held, so they count only when flagged.
  const has = cell.hasExtendedAttrs?.() ?? true;
  const ext = has ? (cell.extended?._ext ?? 0) : 0;
  const urlId = has ? (cell.extended?._urlId ?? 0) : 0;
  const uri = urlId ? term._core?._oscLinkService?.getLinkData?.(urlId)?.uri ?? "" : "";
  return { style: (ext >> 26) & 7, colorMode: ext & 0x3000000, color: ext & 0xffffff, uri };
}

function sgr(cell) {
  const ext = extended(cell);
  const params = ["0"];
  for (const [on, code] of [
    [cell.isBold(), "1"],
    [cell.isDim(), "2"],
    [cell.isItalic(), "3"],
    [cell.isBlink(), "5"],
    [cell.isInverse(), "7"],
    [cell.isInvisible(), "8"],
    [cell.isStrikethrough(), "9"],
    [cell.isOverline(), "53"],
  ]) {
    if (on) params.push(code);
  }
  if (cell.isUnderline() && ext.style) {
    params.push(`4:${ext.style}`);
  } else if (cell.isUnderline() && !ext.uri) {
    params.push("4");
  }
  const underlined = cell.isUnderline() && (ext.style || !ext.uri);
  if (!underlined) {
    // An underline's colour shows only with the underline.
  } else if (ext.colorMode === 0x3000000) {
    params.push(`58;2;${(ext.color >> 16) & 255};${(ext.color >> 8) & 255};${ext.color & 255}`);
  } else if (ext.colorMode) {
    params.push(`58;5;${ext.color & 255}`);
  }
  params.push(...color(cell, cell.isFgRGB(), cell.isFgPalette(), cell.getFgColor(), cell.getFgColorMode(), 30, 90, 38));
  params.push(...color(cell, cell.isBgRGB(), cell.isBgPalette(), cell.getBgColor(), cell.getBgColorMode(), 40, 100, 48));
  return `\x1b]8;;${ext.uri}\x1b\\\x1b[${params.join(";")}m`;
}

function screen() {
  const buffer = term.buffer.active;
  const lines = [];
  let cell;
  for (let y = 0; y < rows; y++) {
    const line = buffer.getLine(buffer.viewportY + y);
    let text = "";
    let last = "";
    for (let x = 0; line && x < cols; x++) {
      cell = line.getCell(x, cell);
      if (!cell || cell.getWidth() === 0) continue;
      const style = sgr(cell);
      if (style !== last) {
        text += style;
        last = style;
      }
      text += cell.getChars() || " ";
    }
    lines.push(`${text}\x1b]8;;\x1b\\\x1b[0m`);
  }
  const hidden = term._core?.coreService?.isCursorHidden ?? false;
  return {
    ansi: lines.join("\n"),
    cursor: [Math.min(buffer.cursorX, cols - 1), buffer.cursorY],
    visible: !hidden,
  };
}

createInterface({ input: process.stdin }).on("line", (line) => {
  const request = JSON.parse(line);
  if (request.read) {
    // An empty write calls back once everything written before it is parsed.
    term.write("", () => reply(screen()));
  } else if (request.send !== undefined) {
    child.write(Buffer.from(request.send, "hex"));
    reply({ sent: true });
  } else if (request.mouse !== undefined) {
    // xterm.js's buttons and actions, from its CoreMouseService.
    const button = { left: 0, middle: 1, right: 2, up: 4, down: 4 }[request.button];
    const action = request.mouse === "scroll" ? (request.button === "up" ? 0 : 1) : request.mouse === "press" ? 1 : 0;
    term._core.coreMouseService.triggerMouseEvent({
      col: request.x, row: request.y, x: 0, y: 0, button, action, ctrl: false, alt: false, shift: false,
    });
    reply({ sent: true });
  } else if (request.quit) {
    child.kill();
    process.exit(0);
  }
});

reply({ ready: true });
