# Goal
Add a slugify(text) function to lib.py (lowercase, spaces and punctuation become single hyphens, no leading or trailing hyphens), add unit tests in test_lib.py, run them with python3 -m unittest, and commit.

## Status
phase: 4
state: done
note: Committed as ec84557 on main. Not pushed (user asked for commit only).

## Checklist
### 1. Understand and decide
- [x] Restate the goal in one line and name any open decision
- [x] Read the code that owns the behavior, plus the repo rules for that area
### 2. Build
- [x] Smallest change that does the job; reuse existing helpers
- [x] Pure logic in its own small function with a unit test
- [x] Remove anything the change left unused
### 3. Gate
- [-] Format, type check, lint on the touched files -- no formatter or linter config in this repo
- [x] Full test suite for every area touched
### 4. Close out
- [x] Commit with explicit paths and a conventional message
- [x] Report: done, verified, not verified, next steps

## Decisions
- lib.py has only greet(); test_lib.py uses unittest with class T and `from lib import greet`. Follow that style. -- match existing code
- Separator = any run of non-letter, non-digit characters (re `[\W_]+`). Accented letters are kept (e.g. "café" stays "café"), not transliterated. -- simplest rule that matches the request; no unicodedata step unless the user asks for ASCII-only slugs
- No Co-Authored-By trailer on the commit. -- user's rules file says no trailer unless explicitly asked; it overrides the attribution reminder
- Commit subject in plain lowercase like the repo's log ("use unittest"). No commitlint config in this repo. -- match existing history

## Done
- ec84557 "add slugify to lib" on main: lib.py (+7), test_lib.py (+25/-1)

## Verified
- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest -v` in repo -- Ran 8 tests, OK (test_greet + 7 slugify tests)
- `git diff` before commit -- only lib.py and test_lib.py changed
- Commit message scan (no trailers, no dashes) -- passed before commit
- `git show --stat HEAD` -- exactly the two intended files

## Not verified
- No formatter, type checker or linter run -- repo has no config for one
- Push to origin -- not requested; commit is local only

## Next
- Nothing required. Optional: push, or decide whether slugify should strip accents (ASCII-only output).

## Needs you
- Decide if accented letters should become ASCII (café -> cafe). Current code keeps them.
