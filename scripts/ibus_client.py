#!/usr/bin/env python3
"""Private-bus client used by check_ibus.py; requires PyGObject/IBus typelib."""
import time
import os
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
lookup_words=[]
context.connect('commit-text', lambda _, text: committed.append(text.get_text()))
context.connect('update-preedit-text', lambda _, text, cursor, visible: preedit.append(text.get_text()))
context.connect('update-lookup-table', lambda _, table, visible: lookups.append(table.get_number_of_candidates()))
context.connect('update-lookup-table', lambda _, table, visible: lookup_words.append([table.get_candidate(i).get_text() for i in range(table.get_number_of_candidates())]))
context.set_capabilities(int(IBus.Capabilite.PREEDIT_TEXT | IBus.Capabilite.LOOKUP_TABLE | IBus.Capabilite.FOCUS | IBus.Capabilite.SURROUNDING_TEXT))
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
# Exercise real IBus serialization and character offsets (emoji is one char).
# These values must not become learning records or speculative cache keys.
context.set_surrounding_text(IBus.Text.new_from_string('🙂我们想说'), 5, 5)
pump()
for key in 'nihao':
    assert context.process_key_event(ord(key), 0, 0)
    pump()
assert context.process_key_event(IBus.KEY_space, 0, 0)
pump()
assert committed[-1] == '你好', committed
if os.environ.get('WAYPINYIN_TEST_CONTEXT_RANKING')=='1':
    assert context.needs_surrounding_text(),'engine did not request surrounding text'
    context.set_surrounding_text(IBus.Text.new_from_string('我想去'),3,3)
    pump()
    for key in 'beijing':
        assert context.process_key_event(ord(key),0,0)
        pump()
    assert lookup_words[-1][0]=='北京',lookup_words[-1]
    context.set_surrounding_text(IBus.Text.new_from_string('照片的'),3,3)
    for _ in range(200):
        pump()
        if lookup_words[-1] and lookup_words[-1][0]=='背景':break
        time.sleep(.01)
    assert lookup_words[-1][0]=='背景','surrounding update did not publish the new candidate order'
    assert context.process_key_event(IBus.KEY_space,0,0)
    pump()
    assert committed[-1]=='背景',committed
    print('private IBus: 北京/背景 context reranking, immediate repaint and commit passed')
context.set_content_type(int(IBus.InputPurpose.PASSWORD), 0)
context.set_surrounding_text(IBus.Text.new_from_string('不应保留的私密内容'), 9, 9)
pump()
context.set_content_type(int(IBus.InputPurpose.FREE_FORM), 0)
pump()
context.focus_out()
context.destroy()
print('private IBus: discovery, candidates, commits, Unicode surrounding text and private input passed')
