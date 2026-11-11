#!/usr/bin/env python3
"""Runs a program in a VTE terminal for the harness, which talks to it in JSON lines.

    python3 harness-vte.py COLS ROWS CWD PROGRAM [ARGS...]

VTE is the engine of GNOME Terminal, Tilix and xfce4-terminal. The terminal draws into an
offscreen window, so nothing shows on the display, though GTK still needs one (Xvfb on CI).
It writes {"ready": true} once the program runs, then answers each line it reads:

    {"read": true}                    {"html": the screen as VTE's HTML, "cursor": [x, y], "size": [c, r]}
    {"send": "1b5b41"}                bytes, in hex, to the program as if typed
    {"key": "Up", "mods": ["ctrl"]}   a key press through VTE's own key encoder
    {"mouse": "press", "button": "left", "x": 3, "y": 2}
                                      a mouse event over a cell counted from 0, through VTE's
                                      own mouse reports. The wheel is "scroll" with "up" or "down".
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

BUTTONS = {"left": 1, "middle": 2, "right": 3}

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

    # Event times, in milliseconds.
    stamp = [0]

    def key(name, mask):
        """A key's press and release as GTK would deliver them. GTK's own test helper can't send
        keys to an offscreen window."""
        # GDK names no keysym for most characters, which come as Unicode keysyms (U00E9).
        if len(name) > 2 and name[0] == "U" and all(c in "0123456789ABCDEF" for c in name[1:]):
            keyval = Gdk.unicode_to_keyval(int(name[1:], 16))
        else:
            keyval = Gdk.keyval_from_name(name)
        display = Gdk.Display.get_default()
        found, entries = Gdk.Keymap.get_for_display(display).get_entries_for_keyval(keyval)
        for kind in (Gdk.EventType.KEY_PRESS, Gdk.EventType.KEY_RELEASE):
            event = Gdk.Event.new(kind)
            fields = event.key
            fields.window = terminal.get_window()
            fields.keyval = keyval
            fields.state = mask
            if found and entries:
                fields.hardware_keycode = entries[0].keycode
                fields.group = entries[0].group
            stamp[0] += 1000
            fields.time = stamp[0]
            event.set_device(display.get_default_seat().get_keyboard())
            terminal.event(event)

    def event_window():
        # VTE takes mouse events on an input-only window of its own over the widget.
        windows = terminal.get_window().get_children()
        return next((w for w in windows if w.is_input_only()), terminal.get_window())

    def mouse(request):
        """A mouse event as GTK would deliver it, at the middle of the cell."""
        x = padding.left + (request["x"] + 0.5) * terminal.get_char_width()
        y = padding.top + (request["y"] + 0.5) * terminal.get_char_height()
        if request["mouse"] == "scroll":
            event = Gdk.Event.new(Gdk.EventType.SCROLL)
            fields = event.scroll
            fields.direction = Gdk.ScrollDirection.UP if request["button"] == "up" else Gdk.ScrollDirection.DOWN
        else:
            kind = Gdk.EventType.BUTTON_PRESS if request["mouse"] == "press" else Gdk.EventType.BUTTON_RELEASE
            event = Gdk.Event.new(kind)
            fields = event.button
            fields.button = BUTTONS[request["button"]]
        fields.window = event_window()
        fields.x, fields.y = x, y
        # A second apart, so no two presses make a double click.
        stamp[0] += 1000
        fields.time = stamp[0]
        event.set_device(Gdk.Display.get_default().get_default_seat().get_pointer())
        terminal.event(event)

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
            key(request["key"], mask)
            reply({"sent": True})
        elif "mouse" in request:
            mouse(request)
            reply({"sent": True})
        elif "quit" in request:
            Gtk.main_quit()
            return False
        return True

    GLib.io_add_watch(sys.stdin, GLib.IO_IN | GLib.IO_HUP, on_input)
    Gtk.main()


if __name__ == "__main__":
    main()
