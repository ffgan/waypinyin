#!/usr/bin/env python3
"""Private-bus client used by check_ibus.py; requires PyGObject/IBus typelib."""
import time
import gi

gi.require_version('IBus', '1.0')
from gi.repository import GLib, IBus

IBus.init()
bus = IBus.Bus.new()
assert bus.is_connected(), 'private IBus connection failed'
assert 'waypinyin' in [e.get_name() for e in bus.list_engines()], 'engine XML not discovered'
context = bus.create_input_context('waypinyin-packaging-test')
assert context
committed, preedit, lookups = [], [], []
context.connect('commit-text', lambda _, text: committed.append(text.get_text()))
context.connect('update-preedit-text', lambda _, text, cursor, visible: preedit.append(text.get_text()))
context.connect('update-lookup-table', lambda _, table, visible: lookups.append(table.get_number_of_candidates()))
context.set_capabilities(int(IBus.Capabilite.PREEDIT_TEXT | IBus.Capabilite.LOOKUP_TABLE | IBus.Capabilite.FOCUS))
assert bus.set_global_engine('waypinyin'), 'cannot select private engine'
context.focus_in()
context.set_engine('waypinyin')
main = GLib.MainContext.default()
def pump():
    while main.pending():
        main.iteration(False)
for _ in range(100):
    pump()
    engine = context.get_engine()
    if engine and engine.get_name() == 'waypinyin':
        break
    time.sleep(0.05)
else:
    raise RuntimeError('private IBus did not start Waypinyin')
for key in 'nihao':
    assert context.process_key_event(ord(key), 0, 0), key
    pump()
assert context.process_key_event(IBus.KEY_space, 0, 0)
for _ in range(50):
    pump()
    if committed:
        break
    time.sleep(0.02)
assert committed == ['你好'], committed
assert any(preedit), 'no preedit signal'
assert any(lookups), 'no candidate signal'
context.focus_out()
context.destroy()
print('private IBus: engine discovery, preedit, candidates, key events and 你好 commit passed')
