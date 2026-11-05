---
name: tidewatch-chat-composer
description: Use when a message box can't be selected or cursor jumps.
category: native-app
---

A call in the frame loop that moves the cursor or asks for the keyboard runs only when it has something to change. An empty call still moves the cursor and takes focus, so each drag selection is undone on the next frame.

- Known case: the new-tab First message box in `crates/tidewatch/src/new_tab.rs`. Every frame, `crates/tidewatch/src/app/draft.rs:56` handed it the pending pictures from the dev switch, almost always an empty list. Putting in no files now returns early (merged to main as `fd7b3b4c`, PR #350).
- When a box loses its selection or cursor, grep the frame code for insert, focus and cursor calls on that box. Each one must return early on empty input.
- Test the box alone first, in the same mode, with and without a file chip. If it selects fine alone, the fault is a call around it in the frame code. Check those calls before the scroll area, the window title bar or the cursor icon. On 2026-11-04 those three were dead ends.
- To see why a headless drag step fails, print the box's cursor range after each frame. Remove the print before the commit.
- Guard test: a headless frame test drags across real text while an empty insert runs on every frame, then asserts the selection holds. It is `text_dragged_over_in_the_message_box_stays_selected_while_no_file_arrives`. It must fail with the guard removed.
- A screenshot run sends no mouse drag, so it cannot show a selection. Name mouse selection in the built app under Not verified until someone drags in a real window.