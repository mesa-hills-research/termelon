#!/usr/bin/env python3
"""Runs a program in a VTE terminal for the harness, which talks to it in JSON lines.

    python3 harness-vte.py COLS ROWS CWD PROGRAM [ARGS...]

VTE is the engine of GNOME Terminal, Tilix and xfce4-terminal. The terminal draws into an
offscreen window, so nothing shows on the display, though GTK still needs one (Xvfb on CI).
It writes {"ready": true} once the program runs, then answers each line it reads:

    {"read": true}                    {"html": the screen as VTE's HTML, "cursor": [x, y], "size": [c, r]}
    {"send": "1b5b41"}                bytes, in hex, to the program as if typed
    {"key": "Up", "mods": ["ctrl"]}   a key press through VTE's own key encoder
    {"quit": true}                    ends the program and this script
"""

import json
import sys

import gi

gi.require_version("Gdk", "3.0")
gi.require_version("Gtk", "3.0")
gi.require_version("Vte", "2.91")
from gi.repository import Gdk, GLib, Gtk, Vte  # noqa: E402

# xterm's default palette, and black on white, so colours read back as the harness computes them.
PALETTE = [
    "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd", "#00cdcd", "#e5e5e5",
    "#7f7f7f", "#ff0000", "#00ff00", "#ffff00", "#5c5cff", "#ff00ff", "#00ffff", "#ffffff",
]
FOREGROUND = "#000000"
BACKGROUND = "#ffffff"

MODIFIERS = {
    "ctrl": Gdk.ModifierType.CONTROL_MASK,
    "alt": Gdk.ModifierType.MOD1_MASK,
    "shift": Gdk.ModifierType.SHIFT_MASK,
    "super": Gdk.ModifierType.SUPER_MASK,
}


def reply(value):
    sys.stdout.write(json.dumps(value) + "\n")
    sys.stdout.flush()


def main():
    cols, rows = int(sys.argv[1]), int(sys.argv[2])
    cwd, argv = sys.argv[3], sys.argv[4:]

    terminal = Vte.Terminal()
    terminal.set_scrollback_lines(0)
    terminal.set_bold_is_bright(False)

    def rgba(spec):
        color = Gdk.RGBA()
        color.parse(spec)
        return color

    terminal.set_colors(rgba(FOREGROUND), rgba(BACKGROUND), [rgba(spec) for spec in PALETTE])
    window = Gtk.OffscreenWindow()
    window.add(terminal)
    window.show_all()
    # The grid's size follows the widget's, which has the style's padding around the cells.
    padding = terminal.get_style_context().get_padding(Gtk.StateFlags.NORMAL)
    width = terminal.get_char_width() * cols + padding.left + padding.right
    height = terminal.get_char_height() * rows + padding.top + padding.bottom
    terminal.set_size_request(width, height)
    while Gtk.events_pending():
        Gtk.main_iteration()

    def spawned(_terminal, _pid, error, *_data):
        if error:
            reply({"error": str(error)})
            Gtk.main_quit()
        else:
            reply({"ready": True})

    terminal.spawn_async(
        Vte.PtyFlags.DEFAULT, cwd, argv, None, GLib.SpawnFlags.SEARCH_PATH, None, None, -1, None, spawned
    )

    def on_input(_source, _condition):
        line = sys.stdin.readline()
        if not line:
            Gtk.main_quit()
            return False
        request = json.loads(line)
        if "read" in request:
            html = terminal.get_text_format(Vte.Format.HTML)
            column, row = terminal.get_cursor_position()
            # Rows count from the start of the buffer, which an erase of the screen moves on.
            top = int(terminal.get_vadjustment().get_value())
            reply({"html": html, "cursor": [column, row - top], "size": [terminal.get_column_count(), terminal.get_row_count()]})
        elif "send" in request:
            terminal.feed_child(bytes.fromhex(request["send"]))
            reply({"sent": True})
        elif "key" in request:
            mask = Gdk.ModifierType(0)
            for name in request.get("mods", []):
                mask |= MODIFIERS[name]
            Gtk.test_widget_send_key(terminal, Gdk.keyval_from_name(request["key"]), mask)
            reply({"sent": True})
        elif "quit" in request:
            Gtk.main_quit()
            return False
        return True

    GLib.io_add_watch(sys.stdin, GLib.IO_IN | GLib.IO_HUP, on_input)
    Gtk.main()


if __name__ == "__main__":
    main()
