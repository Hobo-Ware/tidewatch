# Goal
Add a slugify(text) function to lib.py (lowercase, spaces and punctuation become single hyphens, no leading or trailing hyphens), add unit tests in test_lib.py, run them with python3 -m unittest, and commit.

## Status
phase: 3
state: working
note: slugify and 8 tests written. Running python3 -m unittest next.

## Checklist
### 1. Understand and decide
- [x] Restate the goal in one line and name any open decision
- [x] Read the code that owns the behavior, plus the repo rules for that area
### 2. Build
- [x] Smallest change that does the job; reuse existing helpers
- [x] Pure logic in its own small function with a unit test
- [ ] Remove anything the change left unused
### 3. Gate
- [ ] Format, type check, lint on the touched files
- [ ] Full test suite for every area touched
### 4. Close out
- [ ] Commit with explicit paths and a conventional message
- [ ] Report: done, verified, not verified, next steps

## Decisions
- lib.py has only greet(); test_lib.py uses unittest with class T and `from lib import greet`. Follow that style. -- match existing code
- Separator = any run of non-letter, non-digit characters (re `[\W_]+`). Accented letters are kept (e.g. "café" stays "café"), not transliterated. -- simplest rule that matches the request; no unicodedata step unless the user asks for ASCII-only slugs
- No Co-Authored-By trailer on the commit. -- user's rules file says no trailer unless explicitly asked; it overrides the attribution reminder
- Commit subject in plain lowercase like the repo's log ("use unittest"). No commitlint config in this repo. -- match existing history

## Verified
- (none yet)

## Not verified
- (none yet)

## Next
- Run python3 -m unittest; check git status; stage by name; commit.

## Needs you
- Nothing. Open choice (accent handling) is noted in the final report.
