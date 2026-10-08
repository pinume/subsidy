# Issue tracker: GitHub

Issues and specs live in GitHub Issues for `pinume/subsidy`.
Use the `gh` CLI from the repository root.

## Conventions

- Create: `gh issue create --title "..." --body-file <file>`
- Read: `gh issue view <number> --comments`
- List: `gh issue list --state open --json number,title,body,labels`
- Comment: `gh issue comment <number> --body-file <file>`
- Add labels: `gh issue edit <number> --add-label "..."`
- Remove labels: `gh issue edit <number> --remove-label "..."`
- Close: `gh issue close <number> --comment "..."`

Write multiline bodies to a temporary file and use `--body-file`.
Infer the repository from the Git remote.

## Pull requests as a triage surface

**PRs as a request surface: no.**

## Skill operations

“Publish to the issue tracker” means create a GitHub issue.
“Fetch the relevant ticket” means read the issue and its comments.

## Wayfinding operations

- Map: one issue labelled `wayfinder:map`.
- Child tickets: link as GitHub sub-issues. If unavailable, use a
  task list in the map and `Part of #<map>` in each child.
- Ticket types: `wayfinder:research`, `wayfinder:prototype`,
  `wayfinder:grilling`, or `wayfinder:task`.
- Blocking: use native GitHub issue dependencies with the blocker's
  database ID. If unavailable, record `Blocked by: #<number>`.
- Frontier: choose the first open child in map order with no open
  blockers and no assignee.
- Claim: `gh issue edit <number> --add-assignee @me`.
- Resolve: comment with the decision, close the child, and append
  the decision summary and link to the map.
