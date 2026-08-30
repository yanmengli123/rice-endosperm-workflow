# Context usage panel — implementation notes

Shipped the dock / float / resize behavior from `docs/superpowers/plans/2026-08-14-context-usage-panel.html` in this worktree. Panel content and the Rust backend are unchanged.

## What landed

- Docked open sits in the composer column above `.composer-inner`, matching dialog width and taking at most 46vh. The transcript shrinks instead of sitting behind the card.
- The invisible `.context-usage-backdrop` is gone. A docked panel dismisses on an outside click, Escape, the × / gauge toggle, or a conversation switch; that click still activates the target (caret, inspector, session).
- Dragging the header more than 8px undocks a floating window. The dock button and a header double-click re-dock. No magnetic snap.
- Floating stays open while typing or clicking elsewhere. Escape still closes the topmost surface first.
- Corner-grip resize, 320×220 minimum, clamped to the window. Bar, rows, and expanded details reflow; detail max-height follows panel height.
- Geometry (`wisp-context-usage-x/y/w/h`) persists in localStorage and is clamped to the live viewport on restore. Last mode is in-memory: a restart opens docked.

## Deviations

- **Docked slot lives inside `.composer`, not as a new `.center` sibling.** `.center.split` assigns grid areas to `.chat-stage` and `.composer`. A new sibling would auto-place and break split view. Putting the slot above `.composer-inner` still sits between the transcript and the input, matches composer width, and grows the composer so the flex `chat-stage` shrinks. Conservative: no grid rewrite.
- **Last mode is not written to localStorage.** B6 says a restart always opens docked. Persisting a mode we then ignore on load would be unused state. Geometry is persisted; mode stays on the in-memory signal.
- **Design QA screenshots were not captured.** Dark theme and <960px behavior are covered by Playwright clamps and existing token-based styling. No new PNG baselines were added to `docs/design-qa/`.
- **Scroll-restore coverage asserts the latest reply stays in view** rather than a raw `scrollTop` equality. Opening/closing the docked slot changes scroller height, so a pinned-to-bottom `scrollTop` is not a stable number.
- **Docked open calls `force_chat_bottom()` after a 0ms timeout**, not `schedule_chat_follow()` immediately. The follow helper runs before Leptos commits the slot, so the scroller would snap at the old height and leave the latest reply under the panel. Close and undock still use `schedule_chat_follow()`.
- **Floating enter animation is opacity-only (`motion-fade-in`).** The shared `motion-surface-in` scale/translate would offset a `left`/`top` window by several pixels during the animation.
- **Header mousedown does not `preventDefault`, and the drag overlay mounts only after the 8px threshold.** An immediate overlay (the usual resizer pattern) swallowed the second click of a header double-click, so the explicit re-dock gesture never fired. Window-level move/up listeners track the pre-threshold pointer.

## Files

- `ui/src/main.rs` — signals, drag/resize overlays, docked slot, floating host, outside-click listener
- `ui/src/app_support/prefs.rs` — geom type, clamp, localStorage
- `ui/src/app_support/messages.rs` — `compose_icon("dock")`
- `ui/src/i18n.rs` — dock / resize strings (En + Zh)
- `ui/src/styles/chat.css` — docked/floating/resize; backdrop removed
- `ui-tests/tests/ui.spec.ts` — dock, click-through, float, resize, narrow window
- `docs/ui-design-principles.md`, `design-qa.md`
