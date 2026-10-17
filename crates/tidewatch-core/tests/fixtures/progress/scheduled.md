# Goal
Switch off the old Alpha webhook by Nov 1, then clear the follow-up items.

## Status
phase: 1 (switch off the old webhook)
state: working
note: Step 1a (route check) is in progress now; Nov 1 is then just the merge.

## Scheduled
- [ ] 2026-11-01 -- Switch off the old webhook (#101) -- the main remaining milestone
  - [~] Check that the new /alpha/* route does not catch the old /alpha/server/sync
  - [ ] Mark #101 ready and merge it
  - [ ] Watch the first webhook-written watches and ratings for the webhook-only users (9 so far)
  - [ ] Watch the old address traffic for a week, then remove the old webhook code
- [ ] on: Beta sync is close to shipping -- Beta labels -- must land before Beta sync ships; nothing breaks before then
  - [ ] Add beta to the SDK response list and publish the SDK
  - [ ] Teach web to show the Beta label
  - [ ] Teach iOS to show the Beta label
  - [ ] Teach Android to show the Beta label
- [ ] ~2026-11-07 -- Count which apps still send available_on=alpha -- we need the count before we stop accepting it
- [ ] on: next SDK PR needs bot approval -- Install the review bot on the API repo -- small and optional; without it, SDK PRs cannot get bot approval and must be merged by hand
- [ ] parked: until collection writes move off the old backend -- Stop writing synced items into the collection table (#102) -- waits on #201 and #202

## Next
- Check that the new /alpha/* route does not catch the old /alpha/server/sync (step 1a).

## Needs you
- Nothing needs you today.
