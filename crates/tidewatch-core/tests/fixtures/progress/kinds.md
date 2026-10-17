# Goal
Retire the legacy export job and tidy what it leaves behind.

## Status
phase: 2 Build
state: waiting
note: Waiting on the owner to pick the retention period.

## Checklist
### 1. Understand and decide
- [x] Restate the goal in one line and name any open decision
- [x] Measure first on real data, read-only: counts, timings, who is affected
### 2. Build
- [x] A fresh worktree from the main branch
- [~] The smallest change that does the job
- [ ] Remove anything the change left unused
- [!] Run the migration on staging -- the staging database is locked
### 3. Gate
- [-] Format, type check and lint on the touched files -- no tooling in this repo
- [-] Run the browser suite -- no UI touched

## Decisions
- Keep the old table for one release -- a rollback needs it
- Run `cargo test -- --nocapture` in CI -- the log shows which case failed

## Verified
- Row counts match before and after -- `select count(*)` gave 41,207 on both

## Not verified
- Behavior on the Linux build -- no Linux machine here

## Next
- Write the migration

## Needs you
- Pick the retention period for the audit rows: 30 or 90 days

## Scheduled
- [ ] 2026-10-24 -- Check the first weekly export after the switch -- the job runs on Fridays
  - [ ] Compare the row count with the old report
  - [ ] Mail the owner the numbers
- [ ] ~2026-11-14 -- Remove the old table -- one release after the switch
- [ ] on: the partner confirms the new feed -- Delete the legacy endpoint -- the partner still reads it
- [ ] parked: until the billing rewrite lands -- Drop the invoice columns -- they are read by the old billing code
- [ ] you -- Choose whether the audit rows are kept for 30 or 90 days -- only the owner can decide this
