# Scheduled Automations Design (v0)

Date: 2026-08-15
Status: Implemented (v0)

## Problem

Users want recurring agent work — e.g. a daily literature or progress summary —
without manually opening a session and typing the same prompt. The app needs a
durable trigger that runs a prompt (optionally bound to a skill) on a schedule.

## Scope decision

v0 fires **only while the app is running**; there is no system-level daemon
(launchd / Task Scheduler / systemd). Slots missed while the app was closed
collapse into **one catch-up fire** on next launch. This keeps v0 free of
platform-specific registration code and cross-platform test burden.

## Data model (wisp-store, migration `0048_schedules`)

- `schedules`: `id`, `project_id` (FK, cascade), `frame_id` (nullable — `NULL`
  means a fresh session per fire; deliberately not an FK so a schedule survives
  its target session being deleted), `name`, `prompt`, `skill` (nullable skill
  name), `interval_secs`, `enabled`, `next_run_at`, `last_run_at`,
  `created_at`, `updated_at`. Index `(enabled, next_run_at)` for the due scan.
- `schedule_runs`: one row per fire — `id`, `schedule_id` (FK, cascade),
  `frame_id`, `status` (`fired` | `failed`), `error`, `fired_at`.

## Semantics

- Slot math is anchored: `next_slot_after(anchor, interval, now)` returns the
  smallest `anchor + k*interval` strictly greater than `now`, so an 08:00 daily
  schedule stays at 08:00 regardless of poll timing or catch-ups.
- Firing is claimed atomically (`UPDATE ... WHERE id=? AND next_run_at=? AND
  enabled=1`) so a slow turn or concurrent tick can never double-fire a slot.
- Each fire is a normal agent turn via `send_message_inner` (same path as
  channel messages and delegation auto-resume): it persists to the transcript,
  streams through existing turn events, and queues behind a running turn via
  the per-session workflow lock. The prompt is prefixed with
  `[Scheduled task: <name>]` for provenance.
- An optional skill is attached as a `ComposerReferenceArg::Skill`, rendered
  inline exactly like a user-attached skill; a missing/disabled skill fails the
  run and is recorded in `schedule_runs`.
- Minimum interval is 60s; creation with a past `start_at` makes the schedule
  immediately due (catch-up on the first tick, ~5s after app setup).

## Tauri commands

`create_schedule`, `list_schedules`, `list_schedule_runs`,
`set_schedule_enabled`, `delete_schedule`, `run_schedule_now` (manual,
out-of-cadence fire that leaves `next_run_at` untouched).

## Deliberate exclusions (follow-ups)

- No UI surface yet; commands are ready for a schedules panel.
- No cron expressions / calendar times — interval + anchor only.
- No system-level (app-closed) firing.
- No workflow targets — prompt/skill only; workflow binding can reuse the same
  tables later.
